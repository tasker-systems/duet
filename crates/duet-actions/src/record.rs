//! `duet record`: a take from the machine's microphone, kept as a FLAC file that `duet capture`
//! reads like any other.
//!
//! Writing the file is `duet-capture`'s [`TakeWriter`], and tested there. Opening the input
//! device runs only on a machine with one: no test in CI covers it. What happens to each buffer
//! after the device hands it over (`Handoff`, `Take`) is tested here, and what is refused
//! before a device is opened is tested end to end.
//!
//! The device's callback never waits: it hands each buffer to the writing thread through a
//! bounded channel. Input is lost two ways, and both are written as silence where it was, so
//! what follows keeps its time, and said with when and how much: a buffer the channel cannot
//! take, and a gap the device leaves, measured from the capture time it stamps each buffer with.
//!
//! With `--song`, in a project, the same run plays the song's click on the default output
//! (`click`), and places the take on the song's bars by it (ruling 17). Every buffer's `Stamp`
//! leaves its callback through a bounded channel: the input's with its buffer to the writing
//! thread, the output's to a collector of its own (one the channel cannot take is counted, never
//! waited for). Off the audio threads, each stream's stamps are fit with a line, and from the two
//! lines come where bar 1's tick played in the take's frames and how far the clocks drifted. The
//! click is only ever played: it is never written into the take.
//!
//! With `--calibrate` (ruling 18) the same two streams play the calibration click and record
//! channel 0 into memory; `calibrate` finds the ticks and keeps the pair's measured offset in
//! `calibration.toml`, which `--song` applies to its placement. The speakers-on check of both
//! runs on a machine with devices, opted into by `DUET_E2E_SPEAKERS`
//! (`crates/duet-e2e/tests/placing.rs`).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::Context;
use crate::take::{at, span};
use anyhow::{Context as _, Result, anyhow, bail};
use calibrate::{Calibration, Calibrations, Pair};
use clap::Args;
use click::{ClickVoice, Fit, Line, Stamp, Ticks, clock_ppm, frames_after};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};
use duet_capture::{Depth, TakeWriter, Written};
use duet_core::Position;
use duet_schemas::journal::At;
use duet_schemas::project::Name;
use duet_schemas::song::Placed;
use schemars::JsonSchema;

mod calibrate;
mod click;

/// Buffers waiting for the writer before the device's next one is counted as lost: at a
/// typical 512 frames a buffer, about seven seconds at 48 kHz.
const QUEUE: usize = 640;

/// The silence the click's output starts with before its count-in, in seconds: room for the
/// output to settle before the first tick.
const LEAD_SECS: f64 = 0.2;

/// How long the click runs after bar 1 when no `--seconds` ends the take: four hours.
const CLICK_UNTIL_SECS: f64 = 4.0 * 3600.0;

/// The file in the user config directory each device pair's calibration is kept in.
const CALIBRATION_FILE: &str = "calibration.toml";

#[derive(Debug, Args, JsonSchema)]
pub struct RecordArgs {
    /// The FLAC file to write the take to. It must not exist: a take never replaces a file. In
    /// a project, a take's name instead (lowercase words joined by hyphens, no .flac): the take
    /// is recorded into takes/TAKE/TAKE.flac and added to the project.
    #[arg(required_unless_present = "calibrate")]
    output: Option<PathBuf>,
    /// The input device to record from: its name, or a part of it only one input's name
    /// holds. Without it, the system's default input.
    #[arg(long)]
    device: Option<String>,
    /// Stop after this many seconds, instead of at Ctrl-C.
    #[arg(long, value_parser = positive_seconds)]
    seconds: Option<f64>,
    /// In a project, the song the take is recorded to. Its click plays from the default output:
    /// one bar of count-in in its first meter, then from bar 1 at its tempo and meter, the
    /// downbeats accented. The take is linked to the song and placed by the click at 1.1.0, from
    /// the two streams' timestamps, with how far the two clocks drifted. About 25 ppm of the
    /// drift goes unmeasured on the owner's Mac mini, so the placement holds 2 ms for roughly
    /// the first 80 s. The click is never written into the take; from speakers it is heard in
    /// it, and headphones keep it out.
    #[arg(long)]
    song: Option<Name>,
    /// With --song, leaves the clocks' measured drift out of the placement. It is still stated.
    #[arg(long, requires = "song")]
    no_reconcile: bool,
    /// Measure the offset between the input (the default, or --device) and the default output,
    /// which --song then applies: the click plays 12 ticks from the speakers, the input hears
    /// them, and where they are heard against where the timestamps put them is kept for the pair
    /// in calibration.toml in the user config directory. Inside or outside a project; nothing is
    /// written to a take.
    #[arg(long, conflicts_with_all = ["output", "song", "seconds", "no_reconcile"])]
    calibrate: bool,
}

fn positive_seconds(text: &str) -> std::result::Result<f64, String> {
    match text.parse::<f64>() {
        Ok(secs) if secs.is_finite() && secs > 0.0 => Ok(secs),
        _ => Err(format!("{text:?} is not a number of seconds above zero")),
    }
}

/// Input lost before the take's frame `at`, and written there as `frames` of silence.
#[derive(Debug, PartialEq, Eq)]
struct Loss {
    at: u64,
    frames: u64,
}

/// One buffer from the device, with how many frames were lost just before it.
struct Block {
    lost_before: u64,
    samples: Vec<f32>,
    /// Where the buffer starts on the device's timeline, lost frames included, and the instant
    /// it was captured: what the take is placed against the click by.
    stamp: Stamp,
}

/// What the device's callbacks share with the writing thread.
#[derive(Default)]
struct Shared {
    /// Frames of the device's timeline so far, lost ones included: where an overrun happened.
    delivered: AtomicU64,
    /// Frames lost since the last buffer the channel took.
    lost: AtomicU64,
    /// Where the device reported overruns, in frames of its timeline.
    overruns: Mutex<Vec<u64>>,
    /// Why the stream ended, when the device ended it.
    failed: Mutex<Option<String>>,
}

pub fn run(ctx: &Context, args: RecordArgs) -> Result<()> {
    if args.calibrate {
        return calibrate(ctx, args.device.as_deref());
    }
    let mut output = args
        .output
        .clone()
        .ok_or_else(|| anyhow!("duet record needs a take's name or a .flac file to record to"))?;
    // Every refusal comes before any device is opened.
    let song = match &args.song {
        Some(song) => Some(song_of(ctx, song, &output)?),
        None => None,
    };
    // The pair's calibration is read now, so a file that does not read is refused here.
    let calibrations = match &song {
        Some(_) => match calibration_file()? {
            Some(path) => Calibrations::read(&path)?,
            None => Calibrations::default(),
        },
        None => Calibrations::default(),
    };
    let into = into_project(ctx, &output)?;
    if let Some((_, _, path)) = &into {
        output.clone_from(path);
    }
    if into.is_none() {
        refuse_output(&output)?;
    }
    let stop = catch_ctrl_c()?;
    let host = cpal::default_host();
    let input = open_input(&host, args.device.as_deref())?;
    let (name, rate, channels) = (input.name.clone(), input.rate, input.channels);
    let depth = depth_of(input.format)?;
    // The click's output is built before the file exists too.
    let click = match &song {
        Some((_, _, body)) => {
            let until = args.seconds.unwrap_or(CLICK_UNTIL_SECS);
            Some(open_click(&host, |rate| {
                let lead = (LEAD_SECS * f64::from(rate)).round() as u64;
                Ticks::of(&body.composition, rate, lead, until)
            })?)
        }
        None => None,
    };
    if let Some((_, _, path)) = &into
        && let Some(dir) = path.parent()
    {
        std::fs::create_dir_all(dir).with_context(|| dir.display().to_string())?;
    }
    let mut writer = TakeWriter::create(&output, rate, channels, depth)?;
    if let Err(e) = start(&input, click.as_ref()) {
        // Nothing was recorded: the empty file is not a take.
        drop(writer);
        let _ = std::fs::remove_file(&output);
        return Err(e);
    }
    let until = match args.seconds {
        Some(secs) => format!("for {secs} s"),
        None => "until Ctrl-C".to_owned(),
    };
    eprintln!(
        "recording {} from {name} ({rate} Hz, {channels} channel{}, {}-bit) {until}",
        output.display(),
        if channels == 1 { "" } else { "s" },
        depth.bits(),
    );
    let mut calibration = None;
    if let (Some((song, _, _)), Some((_, heard, _))) = (&song, &click) {
        say_click(song, &name, heard);
        let pair = Pair {
            input: name.clone(),
            output: heard.name.clone(),
            input_rate: rate,
            output_rate: heard.rate,
        };
        calibration = calibrations.find(&pair).cloned();
        say_calibration(&pair, calibration.as_ref());
    }

    let Input {
        stream,
        shared,
        from_device,
        to_device,
        ..
    } = input;
    let limit = args.seconds.map(|s| (s * f64::from(rate)).round() as u64);
    let mut take = Take::new(limit);
    let kept = |path: &Path| {
        format!(
            "the take up to there is kept in {}, readable up to its last whole frame",
            path.display()
        )
    };
    let ended = match take.record(&mut writer, &from_device, &to_device, &stop, &shared) {
        Ok(ended) => ended,
        Err(e) => {
            drop(stream);
            take.report_losses(rate, &shared);
            return Err(e).context(kept(&output));
        }
    };
    drop(stream);
    // The click stops with the take; what it played is in its stamps.
    let heard = click.map(|(output, heard, collector)| {
        drop(output);
        heard.stopped(collector)
    });
    // What the device delivered before the stream stopped is part of the take.
    let drained = (|| -> Result<()> {
        while let Ok(block) = from_device.try_recv() {
            take.add(&mut writer, block, &to_device)?;
        }
        Ok(())
    })();
    if let Err(e) = drained {
        take.report_losses(rate, &shared);
        return Err(e).context(kept(&output));
    }
    let written = match writer.finish() {
        Ok(written) => written,
        Err(e) => {
            take.report_losses(rate, &shared);
            return Err(e).context(kept(&output));
        }
    };
    eprintln!(
        "wrote {} ({})",
        written.path.display(),
        at(written.frames, rate)
    );
    take.report_losses(rate, &shared);
    report_level(&written, &take);
    if let Some(heard) = &heard {
        heard.report();
    }
    // A take is no document, so this records nothing; it goes through the context as every
    // write does.
    ctx.wrote(&written.path, false)?;
    // In a project the take is added, whether the recording ran to its end or stopped: what
    // was written is kept either way. With --song it is linked to the song, placed when bar 1
    // played within it.
    let captured = take.stamps.line();
    if let Some((project, name, _)) = &into {
        let take = crate::take::add_recorded(ctx, project, name, &written.path)?;
        eprintln!(
            "added take {name}: {}, in {}",
            take.digest,
            crate::PROJECT_FILE
        );
        if let (Some((song, dir, _)), Some(heard)) = (&song, &heard) {
            let found = crate::take::read_project(&project.root)?
                .body
                .take(name)
                .cloned()
                .ok_or_else(|| {
                    anyhow!(
                        "the take {name} was added and {} does not hold it",
                        crate::PROJECT_FILE
                    )
                })?;
            let placed = placement(
                written.frames,
                written.rate,
                captured,
                heard,
                calibration.as_ref().map(|c| c.offset_ms),
                args.no_reconcile,
            );
            crate::song::link_take(ctx, dir, &found, placed)?;
            let path = dir.join(crate::song::SONG_FILE);
            match placed {
                Some(placed) => eprintln!(
                    "{}: placed the take {name} by the click, bar 1.1.0 at {:.6} s ({} frames) \
                     into the take",
                    path.display(),
                    placed.at_secs,
                    (placed.at_secs * f64::from(rate)).round()
                ),
                None => eprintln!(
                    "{}: bar 1 never played within the take; {}",
                    path.display(),
                    crate::song::next_capture(song, name)
                ),
            }
        }
    }
    match ended {
        None => Ok(()),
        Some(why) => bail!("recording stopped: {why}; {}", kept(&written.path)),
    }
}

/// Sets Ctrl-C to raise the flag it returns, so a recording stops and keeps what it has.
fn catch_ctrl_c() -> Result<Arc<AtomicBool>> {
    let stop = Arc::new(AtomicBool::new(false));
    let raise = Arc::clone(&stop);
    ctrlc::set_handler(move || raise.store(true, Ordering::Release)).context("catching Ctrl-C")?;
    Ok(stop)
}

/// The input stream, built and not started, and what the run reads from it: its buffers through
/// `from_device`, handed back for reuse through `to_device`.
struct Input {
    stream: cpal::Stream,
    name: String,
    rate: u32,
    channels: u16,
    format: SampleFormat,
    shared: Arc<Shared>,
    from_device: Receiver<Block>,
    to_device: SyncSender<Vec<f32>>,
}

/// Opens the input `wanted` names (see [`pick_device`]), or the default one, and builds its
/// stream: what a take and a calibration record from.
fn open_input(host: &cpal::Host, wanted: Option<&str>) -> Result<Input> {
    let device = match wanted {
        Some(wanted) => {
            let devices: Vec<cpal::Device> = host
                .input_devices()
                .context("listing the audio inputs")?
                .collect();
            let names: Vec<String> = devices.iter().map(name_of).collect();
            devices
                .into_iter()
                .nth(pick_device(&names, wanted)?)
                .expect("picked among them")
        }
        None => host.default_input_device().ok_or_else(|| {
            anyhow!("no audio input device; connect a microphone, or name one with --device")
        })?,
    };
    let name = name_of(&device);
    let supported = device
        .default_input_config()
        .with_context(|| format!("reading {name}'s input format"))?;
    let format = supported.sample_format();
    let config = supported.config();
    let (rate, channels) = (config.sample_rate, config.channels);
    let shared = Arc::new(Shared::default());
    let (to_writer, from_device) = mpsc::sync_channel::<Block>(QUEUE);
    // Buffers handed back for reuse, so the callback rarely allocates. One the device has no
    // room for is dropped.
    let (to_device, spare) = mpsc::sync_channel::<Vec<f32>>(QUEUE);
    let handoff = Handoff::new(to_writer, spare, rate, channels, Arc::clone(&shared));
    // The stream is built before any file exists, so a device that refuses leaves nothing.
    let stream = match format {
        SampleFormat::F32 => input::<f32>(&device, config, handoff, &shared),
        SampleFormat::I16 => input::<i16>(&device, config, handoff, &shared),
        SampleFormat::U16 => input::<u16>(&device, config, handoff, &shared),
        SampleFormat::I32 => input::<i32>(&device, config, handoff, &shared),
        other => bail!("{name}'s sample format, {other}, is not one duet records"),
    }
    .with_context(|| format!("opening {name} for input"))?;
    Ok(Input {
        stream,
        name,
        rate,
        channels,
        format,
        shared,
        from_device,
        to_device,
    })
}

/// Starts the input, then the click: input first, so the recording holds the click's first tick.
fn start(input: &Input, click: Option<&(cpal::Stream, Heard, Collector)>) -> Result<()> {
    input
        .stream
        .play()
        .with_context(|| format!("starting input from {}", input.name))?;
    if let Some((output, heard, _)) = click {
        output
            .play()
            .with_context(|| format!("starting the click on {}", heard.name))?;
    }
    Ok(())
}

/// `<user config dir>/calibration.toml`, or none when there is no user config directory.
fn calibration_file() -> Result<Option<PathBuf>> {
    Ok(crate::config::Dirs::find()?
        .user
        .map(|(dir, _)| dir.join(CALIBRATION_FILE)))
}

/// Says which calibration `record --song` applies for `pair`, or that it has none.
fn say_calibration(pair: &Pair, calibration: Option<&Calibration>) {
    match calibration {
        Some(c) => eprintln!(
            "calibrated: {:+.2} ms for {} → {}, measured {}",
            c.offset_ms, pair.input, pair.output, c.measured
        ),
        None => eprintln!(
            "not calibrated for {} → {}: the placement is as the devices report their latency, \
             which can be off by milliseconds; duet record --calibrate measures it",
            pair.input, pair.output
        ),
    }
}

/// Now, to the millisecond, as the journal stamps an entry.
fn now() -> At {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
    At::from_unix_ms(ms)
}

/// `duet record --calibrate`: plays the calibration click from the default output while the input
/// records channel 0 into memory, finds each tick against where the streams' lines put it, and
/// keeps the median error for the pair in `calibration.toml`. Stores nothing when the input heard
/// nothing, too few ticks, or ticks too far apart.
fn calibrate(ctx: &Context, device: Option<&str>) -> Result<()> {
    let path = calibration_file()?.ok_or_else(|| {
        anyhow!(
            "there is no user config directory to keep calibration.toml in; set {} to one",
            crate::config::CONFIG_DIR_VAR
        )
    })?;
    // Refused before any device opens when it does not read.
    let mut held = Calibrations::read(&path)?;
    let stop = catch_ctrl_c()?;
    let host = cpal::default_host();
    let input = open_input(&host, device)?;
    let click = open_click(&host, calibrate::ticks)?;
    let pair = Pair {
        input: input.name.clone(),
        output: click.1.name.clone(),
        input_rate: input.rate,
        output_rate: click.1.rate,
    };
    start(&input, Some(&click))?;
    eprintln!(
        "calibrating {pair}: the click plays {} ticks from {} after a second's silence, for the \
         microphone to hear; keep the room quiet",
        calibrate::TICKS,
        pair.output
    );
    let (output, heard, collector) = click;
    let last = heard.ticks.last().copied().unwrap_or(0) as f64 / f64::from(heard.rate);
    let wanted = ((last + calibrate::SEARCH_SECS + 0.5) * f64::from(input.rate)).ceil() as u64;
    let listened = listen(&input, wanted, &stop);
    drop(input.stream);
    drop(output);
    let heard = heard.stopped(collector);
    heard.report();
    let (samples, captured) = listened?;
    let (Some(taken), Some(played)) = (captured.line(), heard.line) else {
        bail!("the streams gave no timestamps to calibrate by; nothing is stored");
    };
    let rate = f64::from(input.rate);
    let expected: Vec<f64> = heard
        .ticks
        .iter()
        .map(|&f| click::frames_after(&taken, &played, f, input.rate) / rate)
        .collect();
    let found = calibrate::heard(&samples, input.rate, &expected).map_err(anyhow::Error::new)?;
    let existed = path.exists();
    held.set(Calibration {
        pair: pair.clone(),
        offset_ms: found.offset_ms,
        ticks: found.ticks,
        spread_ms: found.spread_ms,
        measured: now(),
    });
    held.write(&path)?;
    ctx.wrote(&path, existed)?;
    println!(
        "calibrated {pair}: {:+.2} ms, {} of {} ticks heard, {:.2} ms apart; written to {}",
        found.offset_ms,
        found.ticks,
        expected.len(),
        found.spread_ms,
        path.display()
    );
    Ok(())
}

/// Channel 0 of the input until it holds `frames`, input lost as silence where it was, and the
/// line through its buffers' stamps.
fn listen(input: &Input, frames: u64, stop: &AtomicBool) -> Result<(Vec<f32>, Fit)> {
    let channels = usize::from(input.channels.max(1));
    let mut samples = Vec::with_capacity(usize::try_from(frames).unwrap_or(0));
    let mut stamps = Fit::default();
    while (samples.len() as u64) < frames {
        if stop.load(Ordering::Acquire) {
            bail!("the calibration was stopped before its last tick; nothing is stored");
        }
        if let Some(e) = input.shared.failed.lock().ok().and_then(|f| f.clone()) {
            bail!("the input stopped: {e}; nothing is stored");
        }
        match input.from_device.recv_timeout(Duration::from_millis(20)) {
            Ok(block) => {
                stamps.add(block.stamp);
                let lost = usize::try_from(block.lost_before).unwrap_or(usize::MAX);
                samples.extend(std::iter::repeat_n(0.0, lost));
                samples.extend(block.samples.iter().step_by(channels));
                let _ = input.to_device.try_send(block.samples);
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                bail!("the input stream closed; nothing is stored")
            }
        }
    }
    Ok((samples, stamps))
}

/// The song `--song` names: its name, its folder (`songs/<song>`) and the song. Refused, before
/// any device is opened, outside a project, for an output that is a file's path and not a
/// take's name, for a song the project does not have, naming those it has, and for a song
/// whose composition no click can be played from.
fn song_of(
    ctx: &Context,
    song: &Name,
    output: &Path,
) -> Result<(Name, PathBuf, duet_schemas::Song)> {
    let Some(project) = ctx.project() else {
        bail!(
            "duet record --song places the take on a song in a project, and there is no project \
             here (`duet init` makes one)"
        );
    };
    if take_name(output).is_none() {
        let take = output
            .file_stem()
            .and_then(|s| s.to_str())
            .filter(|s| s.parse::<Name>().is_ok())
            .unwrap_or("<take>");
        bail!(
            "--song takes a take's name (lowercase words joined by hyphens, no .flac), not a \
             file: the take is recorded into takes/<take>/<take>.flac and added to the project, \
             such as `duet record {take} --song {song}`"
        );
    }
    let dir = crate::song::folder(&project.root, song);
    if !dir.join(crate::song::SONG_FILE).exists() {
        let songs: Vec<String> = crate::song::listed(&project.root)?
            .into_iter()
            .filter(|l| l.song.is_ok())
            .map(|l| format!("  {}", l.folder))
            .collect();
        let songs = if songs.is_empty() {
            "  (none; `duet song new` makes one)".to_owned()
        } else {
            songs.join("\n")
        };
        bail!("the project has no song {song}; its songs are:\n{songs}");
    }
    let doc = crate::song::read_song(&dir)?;
    // A composition the click cannot be read from is refused now, not once the devices are open.
    Ticks::of(&doc.body.composition, 48_000, 0, 0.0)
        .with_context(|| format!("{}: the click", dir.join(crate::song::SONG_FILE).display()))?;
    Ok((song.clone(), dir, doc.body))
}

/// What the run knows of the click: the output it plays from, where bar 1's tick is on that
/// output's frames, what its callback reported, and, once it has stopped, the line through its
/// stamps and the frame after its last buffer.
struct Heard {
    name: String,
    rate: u32,
    /// Each tick's output frame.
    ticks: Vec<u64>,
    bar_one: u64,
    played: Arc<Played>,
    line: Option<Line>,
    end: u64,
}

impl Heard {
    /// The click once its stream has stopped: what the collector gathered.
    fn stopped(mut self, collector: Collector) -> Heard {
        collector.done.store(true, Ordering::Release);
        if let Ok((fit, end)) = collector.thread.join() {
            self.line = fit.line();
            self.end = end;
        }
        self
    }

    /// Says what went wrong with the click's output, if anything did.
    fn report(&self) {
        let glitches = self.played.glitches.load(Ordering::Acquire);
        if glitches > 0 {
            eprintln!(
                "the click's output on {} glitched {glitches} time{}: where bar 1 fell is \
                 measured from the output's timestamps, and a glitch before it may move it",
                self.name,
                if glitches == 1 { "" } else { "s" }
            );
        }
        if let Some(e) = self.played.failed.lock().ok().and_then(|f| f.clone()) {
            eprintln!("the click stopped before the take did: {e}");
        }
        let dropped = self.played.dropped.load(Ordering::Acquire);
        if dropped > 0 {
            eprintln!(
                "{dropped} of the click's stamps were not collected in time; its line is drawn \
                 through the rest"
            );
        }
    }
}

/// The thread that gathers the click's stamps off the audio thread, and the flag that ends it
/// once the stream has stopped.
struct Collector {
    thread: JoinHandle<(Fit, u64)>,
    done: Arc<AtomicBool>,
}

impl Collector {
    /// Gathers each `(stamp, frames)` the click's callback sends, until the stream has stopped:
    /// the stamps' fit, and the frame after the last buffer.
    fn start(from_click: Receiver<(Stamp, u64)>) -> Collector {
        let done = Arc::new(AtomicBool::new(false));
        let stop = Arc::clone(&done);
        let thread = std::thread::spawn(move || {
            let (mut fit, mut end) = (Fit::default(), 0);
            let mut take = |(stamp, frames): (Stamp, u64)| {
                fit.add(stamp);
                end = stamp.frame + frames;
            };
            loop {
                match from_click.recv_timeout(Duration::from_millis(20)) {
                    Ok(stamped) => take(stamped),
                    Err(RecvTimeoutError::Timeout) if stop.load(Ordering::Acquire) => break,
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
            while let Ok(stamped) = from_click.try_recv() {
                take(stamped);
            }
            (fit, end)
        });
        Collector { thread, done }
    }
}

/// What the click's output callback shares with the run.
#[derive(Debug, Default)]
struct Played {
    /// Stamps the collector's channel had no room for.
    dropped: AtomicU64,
    /// Underruns the output reported.
    glitches: AtomicU64,
    /// Why the output ended, when it did.
    failed: Mutex<Option<String>>,
}

/// Opens the default output and builds a click's stream on it, its ticks from `ticks_at` at the
/// output's rate: what `--song` and `--calibrate` play. Not started.
fn open_click(
    host: &cpal::Host,
    ticks_at: impl FnOnce(u32) -> Result<Ticks>,
) -> Result<(cpal::Stream, Heard, Collector)> {
    let device = host
        .default_output_device()
        .ok_or_else(|| anyhow!("no audio output device, which the click plays on; connect one"))?;
    let name = name_of(&device);
    let supported = device
        .default_output_config()
        .with_context(|| format!("reading {name}'s output format"))?;
    let config = supported.config();
    let (rate, channels) = (config.sample_rate, config.channels);
    let ticks = ticks_at(rate)?;
    let bar_one = ticks.bar_one;
    let frames = ticks.frames.iter().map(|&(f, _)| f).collect();
    let voice = ClickVoice::new(ticks, rate, channels);
    let played = Arc::new(Played::default());
    let (to_collector, from_click) = mpsc::sync_channel::<(Stamp, u64)>(QUEUE);
    let stream = match supported.sample_format() {
        SampleFormat::F32 => output::<f32>(&device, config, voice, to_collector, &played),
        SampleFormat::I16 => output::<i16>(&device, config, voice, to_collector, &played),
        SampleFormat::U16 => output::<u16>(&device, config, voice, to_collector, &played),
        SampleFormat::I32 => output::<i32>(&device, config, voice, to_collector, &played),
        other => bail!("{name}'s sample format, {other}, is not one duet plays"),
    }
    .with_context(|| format!("opening {name} for the click"))?;
    Ok((
        stream,
        Heard {
            name,
            rate,
            ticks: frames,
            bar_one,
            played,
            line: None,
            end: 0,
        },
        Collector::start(from_click),
    ))
}

/// The output stream that plays the click: each buffer's playback stamp, at the click's frame,
/// sent to the collector without waiting (one it has no room for is counted), then the buffer
/// filled by the voice through a buffer made up front, so the callback rarely allocates. An
/// underrun is counted; any other error ends the click, not the take.
fn output<T: SizedSample + FromSample<f32>>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    mut voice: ClickVoice,
    to_collector: SyncSender<(Stamp, u64)>,
    played: &Arc<Played>,
) -> Result<cpal::Stream> {
    let channels = usize::from(config.channels).max(1);
    let (stamps, errors) = (Arc::clone(played), Arc::clone(played));
    let mut scratch = vec![0.0f32; 8192 * channels];
    let mut frame = 0_u64;
    let mut warned = false;
    device
        .build_output_stream(
            config,
            move |data: &mut [T], info: &cpal::OutputCallbackInfo| {
                let frames = data.len() / channels;
                let stamp = Stamp {
                    frame,
                    nanos: info.timestamp().playback.as_nanos(),
                };
                if to_collector.try_send((stamp, frames as u64)).is_err() {
                    stamps.dropped.fetch_add(1, Ordering::AcqRel);
                }
                let samples = frames * channels;
                if scratch.len() < samples {
                    scratch.resize(samples, 0.0);
                }
                let filled = &mut scratch[..samples];
                voice.fill(filled);
                for (out, &sample) in data.iter_mut().zip(filled.iter()) {
                    *out = T::from_sample(sample);
                }
                // A partial frame at the end, which a device should never send, is silent.
                for out in &mut data[samples..] {
                    *out = T::from_sample(0.0);
                }
                frame += frames as u64;
            },
            move |e| match e.kind() {
                cpal::ErrorKind::Xrun => {
                    errors.glitches.fetch_add(1, Ordering::AcqRel);
                }
                cpal::ErrorKind::RealtimeDenied => {
                    if !warned {
                        eprintln!(
                            "duet: real-time priority refused for the click, still playing: {e}"
                        );
                        warned = true;
                    }
                }
                _ => {
                    if let Ok(mut f) = errors.failed.lock() {
                        *f = Some(e.to_string());
                    }
                }
            },
            None,
        )
        .map_err(Into::into)
}

/// What `record --song` says as it starts: that a click from speakers is heard in the take,
/// that two devices' clocks may drift, and where the offset is unchecked.
fn say_click(song: &Name, input: &str, heard: &Heard) {
    eprintln!(
        "the click of {song} plays on {}: one bar of count-in, then from bar 1; from speakers it \
         is heard in the take, and headphones keep it out",
        heard.name
    );
    if input != heard.name {
        eprintln!(
            "the input ({input}) and output ({}) are different devices: their clocks may drift, \
             and duet measures it",
            heard.name
        );
    }
    if cfg!(not(target_os = "macos")) {
        eprintln!(
            "where bar 1 falls in the take is as this system reports its streams' timing, \
             unchecked here (it is checked on macOS)"
        );
    }
}

/// Where bar 1 fell in a take of `frames` at `rate`, from the two streams' stamps moved by the
/// pair's calibration offset in ms, if it has one, to a whole frame, and the clocks' drift,
/// stated. None when bar 1 never played within the take: the click had not reached it when the take
/// ended, as when Ctrl-C stops it during the count-in.
fn placement(
    frames: u64,
    rate: u32,
    input: Option<Line>,
    heard: &Heard,
    calibration_ms: Option<f64>,
    no_reconcile: bool,
) -> Option<Placed> {
    let (taken, played) = (input?, heard.line?);
    if heard.end <= heard.bar_one {
        return None;
    }
    // Unrounded: the offset is rounded once, to a whole frame, after the calibration's shift.
    let reported = frames_after(&taken, &played, heard.bar_one, rate).max(0.0);
    let shift = calibration_ms.map_or(0.0, |ms| ms / 1000.0 * f64::from(rate));
    let offset = (reported + shift).round();
    if offset < 0.0 || offset >= frames as f64 {
        return None;
    }
    let ppm = clock_ppm(&taken, &played, rate, heard.rate);
    match ppm {
        Some(ppm) => eprintln!(
            "the input's clock ran {ppm:+.1} ppm against the click's over the take{}",
            if no_reconcile {
                "; --no-reconcile leaves it out of the placement"
            } else {
                ", and the placement keeps it as clock_ppm"
            }
        ),
        None => eprintln!(
            "the clocks' drift was not measured: a stream ran under a second, too short to tell \
             its rate, so the placement has no clock_ppm"
        ),
    }
    Some(Placed::click(
        Position::bar(1),
        offset / f64::from(rate),
        if no_reconcile { None } else { ppm },
    ))
}

/// The writing thread's view of the take: what it has written, and what was lost.
struct Take {
    /// The frames `--seconds` allows, if it was given.
    limit: Option<u64>,
    /// Whether the limit has been reached: nothing more is written.
    full: bool,
    losses: Vec<Loss>,
    /// The loudest sample, to tell a silent take from a quiet one.
    peak: f32,
    /// Each buffer's capture stamp, as the writer takes it: the input's clock.
    stamps: Fit,
}

impl Take {
    fn new(limit: Option<u64>) -> Take {
        Take {
            limit,
            full: false,
            losses: Vec::new(),
            peak: 0.0,
            stamps: Fit::default(),
        }
    }

    /// Writes the device's buffers until Ctrl-C, the limit, or the device ends the stream,
    /// which is said as the reason.
    fn record(
        &mut self,
        writer: &mut TakeWriter,
        from_device: &Receiver<Block>,
        spare: &SyncSender<Vec<f32>>,
        stop: &AtomicBool,
        shared: &Shared,
    ) -> Result<Option<String>> {
        loop {
            if self.full || stop.load(Ordering::Acquire) {
                return Ok(None);
            }
            if let Some(e) = shared.failed.lock().ok().and_then(|f| f.clone()) {
                return Ok(Some(e));
            }
            match from_device.recv_timeout(Duration::from_millis(20)) {
                Ok(block) => self.add(writer, block, spare)?,
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    return Ok(Some("the input stream closed".into()));
                }
            }
        }
    }

    /// Writes one buffer, after silence for any input lost before it, up to the limit. The
    /// buffer is handed back for reuse.
    fn add(
        &mut self,
        writer: &mut TakeWriter,
        block: Block,
        spare: &SyncSender<Vec<f32>>,
    ) -> Result<()> {
        // Past the limit too: the device's clock runs on, and the line through it is longer.
        self.stamps.add(block.stamp);
        if !self.full {
            let limit = self.limit;
            let room =
                |writer: &TakeWriter| limit.map_or(u64::MAX, |l| l.saturating_sub(writer.frames()));
            let silence = block.lost_before.min(room(writer));
            if silence > 0 {
                self.losses.push(Loss {
                    at: writer.frames(),
                    frames: silence,
                });
                writer.silence(silence)?;
            }
            let channels = usize::from(writer.channels());
            let frames = (block.samples.len() / channels) as u64;
            let keep = frames.min(room(writer)) as usize * channels;
            let samples = &block.samples[..keep];
            self.peak = samples.iter().fold(self.peak, |p, s| p.max(s.abs()));
            writer.write(samples)?;
            self.full = room(writer) == 0;
        }
        let _ = spare.try_send(block.samples);
        Ok(())
    }

    /// Says what was lost and what the device reported, within the take: input lost after the
    /// limit was never part of it.
    fn report_losses(&self, rate: u32, shared: &Shared) {
        for loss in &self.losses {
            eprintln!(
                "lost {} of input at {}, written as silence so what follows keeps its time",
                span(loss.frames, rate),
                at(loss.at, rate)
            );
        }
        let lost_at_end = shared.lost.load(Ordering::Acquire);
        if lost_at_end > 0 && !self.full {
            eprintln!(
                "lost the last {} of input before recording stopped",
                span(lost_at_end, rate)
            );
        }
        let overruns = shared
            .overruns
            .lock()
            .map(|o| o.clone())
            .unwrap_or_default();
        for frame in overruns
            .into_iter()
            .filter(|&f| self.limit.is_none_or(|l| f < l))
        {
            eprintln!(
                "the device reported an overrun at {}; a gap it left in the input is measured \
                 from its timestamps and listed as lost",
                at(frame, rate)
            );
        }
    }
}

/// Says what the take's level tells the owner: samples at full scale, or silence throughout.
fn report_level(written: &Written, take: &Take) {
    if written.full_scale > 0 {
        eprintln!(
            "{} samples reached full scale, so the take may be clipped; turn the input down or \
             sing further from the microphone",
            written.full_scale
        );
    }
    if written.frames > 0 && take.peak == 0.0 {
        eprintln!(
            "the take is silent throughout; on macOS, the terminal app needs microphone access \
             (System Settings, Privacy & Security, Microphone)"
        );
    }
}

/// Where the device's next buffer should start, from the capture time it stamps each one with.
struct Clock {
    rate: u32,
    /// The capture instant, in nanoseconds of the stream's clock, the next buffer should have.
    next: Option<u128>,
}

impl Clock {
    /// Frames the device left out before a buffer captured at `capture` that holds `frames`:
    /// how far it starts after where the last one ended. Under half a buffer is the timestamps'
    /// jitter, not a gap.
    fn gap(&mut self, capture: u128, frames: u64) -> u64 {
        let per_frame = 1e9 / f64::from(self.rate);
        let gap = match self.next {
            Some(expected) if capture > expected => {
                ((capture - expected) as f64 / per_frame).round() as u64
            }
            _ => 0,
        };
        self.next = Some(capture + (frames as f64 * per_frame).round() as u128);
        if gap * 2 > frames { gap } else { 0 }
    }
}

/// What the device's data callback does with each buffer, apart from the device: converts it
/// into a reused buffer, counts any gap before it, and hands it to the writer without waiting.
struct Handoff {
    to_writer: SyncSender<Block>,
    spare: Receiver<Vec<f32>>,
    /// A buffer the channel refused, kept to reuse: never freed on the audio thread.
    refused: Option<Vec<f32>>,
    clock: Clock,
    channels: u64,
    shared: Arc<Shared>,
}

impl Handoff {
    fn new(
        to_writer: SyncSender<Block>,
        spare: Receiver<Vec<f32>>,
        rate: u32,
        channels: u16,
        shared: Arc<Shared>,
    ) -> Handoff {
        Handoff {
            to_writer,
            spare,
            refused: None,
            clock: Clock { rate, next: None },
            channels: u64::from(channels.max(1)),
            shared,
        }
    }

    /// Hands over one buffer of interleaved `input`, captured at `capture` nanoseconds.
    fn hand_off(&mut self, input: impl ExactSizeIterator<Item = f32>, capture: u128) {
        let frames = input.len() as u64 / self.channels;
        let gap = self.clock.gap(capture, frames);
        let before = self
            .shared
            .delivered
            .fetch_add(gap + frames, Ordering::AcqRel);
        // The buffer starts after the gap: lost frames are on the take's timeline as silence.
        let stamp = Stamp {
            frame: before + gap,
            nanos: capture,
        };
        let mut samples = self
            .refused
            .take()
            .or_else(|| self.spare.try_recv().ok())
            .unwrap_or_default();
        samples.clear();
        samples.extend(input);
        let lost_before = self.shared.lost.swap(0, Ordering::AcqRel) + gap;
        match self.to_writer.try_send(Block {
            lost_before,
            samples,
            stamp,
        }) {
            Ok(()) => {}
            Err(TrySendError::Full(block) | TrySendError::Disconnected(block)) => {
                self.shared
                    .lost
                    .fetch_add(lost_before + frames, Ordering::AcqRel);
                self.refused = Some(block.samples);
            }
        }
    }
}

/// The input stream, each buffer passed through `handoff`. An overrun is noted where it
/// happened; a change of device or any other error ends the take.
fn input<T: SizedSample>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    mut handoff: Handoff,
    shared: &Arc<Shared>,
) -> Result<cpal::Stream>
where
    f32: cpal::FromSample<T>,
{
    let errors = Arc::clone(shared);
    let mut warned = false;
    device
        .build_input_stream(
            config,
            move |input: &[T], info: &cpal::InputCallbackInfo| {
                let capture = info.timestamp().capture.as_nanos();
                handoff.hand_off(
                    input.iter().map(|&s| cpal::Sample::to_sample::<f32>(s)),
                    capture,
                );
            },
            move |e| match e.kind() {
                cpal::ErrorKind::Xrun => {
                    let at = errors.delivered.load(Ordering::Acquire);
                    if let Ok(mut overruns) = errors.overruns.lock() {
                        overruns.push(at);
                    }
                }
                cpal::ErrorKind::RealtimeDenied => {
                    if !warned {
                        eprintln!("duet: real-time priority refused, still recording: {e}");
                        warned = true;
                    }
                }
                // A rerouted input may change rate or channels mid-file: the take ends there.
                cpal::ErrorKind::DeviceChanged => {
                    if let Ok(mut f) = errors.failed.lock() {
                        *f = Some("the input device changed".to_owned());
                    }
                }
                _ => {
                    if let Ok(mut f) = errors.failed.lock() {
                        *f = Some(e.to_string());
                    }
                }
            },
            None,
        )
        .map_err(Into::into)
}

/// Where `duet record <take>` records in a project: the project, the take's name, and
/// `takes/<take>/<take>.flac` under its root. `None` when the output is a file's path, as it
/// is outside a project. A name the project has a take by already is refused, as is a file
/// there already, before any device is opened.
fn into_project(ctx: &Context, output: &Path) -> Result<Option<(crate::Project, Name, PathBuf)>> {
    let Some(name) = take_name(output) else {
        return Ok(None);
    };
    let Some(project) = ctx.project() else {
        bail!(
            "duet record {name} records the take {name} into a project, and there is none \
             here (`duet init` makes one); outside a project, name a FLAC file, such as \
             {name}.flac"
        );
    };
    let doc = crate::take::read_project(&project.root)?;
    crate::take::refuse_name_taken(&doc.body, &name)?;
    let path = project
        .root
        .join(crate::take::TAKES_DIR)
        .join(name.as_str())
        .join(format!("{name}.flac"));
    if path.symlink_metadata().is_ok() {
        bail!(
            "{} exists; a take never replaces a file, so move it or name another take",
            path.display()
        );
    }
    Ok(Some((project.clone(), name, path)))
}

/// The take's name `output` is, when it is one: no extension, no directory, and a name.
fn take_name(output: &Path) -> Option<Name> {
    if output.extension().is_some() || output.components().count() != 1 {
        return None;
    }
    output.to_str()?.parse().ok()
}

/// Refuses an output that exists, is not named `.flac`, or is in a directory that does not
/// exist, before any device is opened.
fn refuse_output(output: &Path) -> Result<()> {
    let flac = output
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("flac"));
    if !flac {
        bail!(
            "duet record writes FLAC; name the take with .flac, such as {}",
            output.with_extension("flac").display()
        );
    }
    if output.symlink_metadata().is_ok() {
        bail!(
            "{} exists; a take never replaces a file, so move it or name another",
            output.display()
        );
    }
    if let Some(dir) = output.parent().filter(|d| !d.as_os_str().is_empty())
        && !dir.is_dir()
    {
        bail!(
            "{} is not a directory, so the take cannot be written there",
            dir.display()
        );
    }
    Ok(())
}

/// The depth a device's samples are kept at: all of a 16-bit sample, or 24 bits of a wider
/// or float one.
fn depth_of(format: SampleFormat) -> Result<Depth> {
    Ok(match format {
        SampleFormat::I16 | SampleFormat::U16 => Depth::Sixteen,
        SampleFormat::F32 | SampleFormat::I32 => Depth::TwentyFour,
        other => bail!("the input's sample format, {other}, is not one duet records"),
    })
}

fn name_of(device: &cpal::Device) -> String {
    device
        .description()
        .map(|d| d.name().to_owned())
        .unwrap_or_else(|_| "an unnamed input".to_owned())
}

/// The input `wanted` names among `names`: the one with exactly that name, or else the one
/// whose name holds it, ignoring case. Refused when none or several do, or when it is empty,
/// so an input is never picked by accident.
fn pick_device(names: &[String], wanted: &str) -> Result<usize> {
    let list = |which: &mut dyn Iterator<Item = &String>| -> String {
        let lines: Vec<String> = which.map(|n| format!("  {n}")).collect();
        if lines.is_empty() {
            "  (none)".to_owned()
        } else {
            lines.join("\n")
        }
    };
    if wanted.trim().is_empty() {
        bail!(
            "--device needs an input's name; the inputs are:\n{}",
            list(&mut names.iter())
        );
    }
    let exact: Vec<usize> = (0..names.len()).filter(|&i| names[i] == wanted).collect();
    let lower = wanted.to_lowercase();
    let found = if exact.is_empty() {
        (0..names.len())
            .filter(|&i| names[i].to_lowercase().contains(&lower))
            .collect()
    } else {
        exact
    };
    match found.as_slice() {
        [one] => Ok(*one),
        [] => bail!(
            "no audio input is named {wanted:?}; the inputs are:\n{}",
            list(&mut names.iter())
        ),
        _ if found.iter().all(|&i| names[i] == wanted) => bail!(
            "{} audio inputs are named {wanted:?}, and duet cannot tell them apart",
            found.len()
        ),
        _ => bail!(
            "{wanted:?} is part of more than one input's name; name one:\n{}",
            list(&mut found.iter().map(|&i| &names[i]))
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::sync::atomic::Ordering;
    use std::sync::mpsc;

    use duet_capture::{Depth, FileSource, Source, TakeWriter};

    use duet_core::Position;
    use duet_schemas::song::Placed;

    use super::click::{Line, Stamp};
    use super::{
        Block, Clock, Handoff, Heard, Loss, Played, Shared, Take, at, pick_device, placement,
        refuse_output, span,
    };

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|&n| n.to_owned()).collect()
    }

    /// A new scratch path, for a take written in a test.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("duet-record-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join(name);
        let _ = std::fs::remove_file(&path);
        path
    }

    fn read_back(path: &Path) -> Vec<f32> {
        let mut source = FileSource::open(path).expect("open");
        let mut samples = Vec::new();
        while let Some(chunk) = source.next_chunk().expect("read") {
            samples.extend(chunk.samples);
        }
        samples
    }

    /// 10 ms at 48 kHz, in nanoseconds.
    const TEN_MS: u128 = 10_000_000;

    #[test]
    fn the_clock_measures_a_gap_the_device_left_and_ignores_jitter() {
        let mut clock = Clock {
            rate: 48_000,
            next: None,
        };
        // 480 frames is 10 ms: buffers on time, then one 0.2 ms late (jitter), then one that
        // starts 20 ms after the last ended (two buffers dropped).
        assert_eq!(clock.gap(0, 480), 0);
        assert_eq!(clock.gap(TEN_MS, 480), 0);
        assert_eq!(clock.gap(2 * TEN_MS + 200_000, 480), 0);
        assert_eq!(clock.gap(5 * TEN_MS + 200_000, 480), 960);
        // Early is never a gap.
        assert_eq!(clock.gap(5 * TEN_MS, 480), 0);
    }

    #[test]
    fn a_refused_buffer_is_counted_and_carried_to_the_next_one_taken() {
        let shared = Arc::new(Shared::default());
        let (to_writer, from_device) = mpsc::sync_channel::<Block>(1);
        let (_to_device, spare) = mpsc::sync_channel::<Vec<f32>>(1);
        let mut handoff = Handoff::new(to_writer, spare, 48_000, 2, Arc::clone(&shared));
        let buffer = [0.5f32; 960]; // 480 stereo frames, 10 ms
        handoff.hand_off(buffer.iter().copied(), 0);
        // The channel is full: this one is lost, and its buffer kept for reuse.
        handoff.hand_off(buffer.iter().copied(), TEN_MS);
        assert_eq!(shared.lost.load(Ordering::Acquire), 480);
        assert!(handoff.refused.is_some());
        let first = from_device.try_recv().expect("the first buffer");
        assert_eq!((first.lost_before, first.samples.len()), (0, 960));
        // The next one taken says what was lost before it; the device also left 10 ms out.
        handoff.hand_off(buffer.iter().copied(), 3 * TEN_MS);
        let third = from_device.try_recv().expect("the third buffer");
        assert_eq!(third.lost_before, 480 + 480);
        assert_eq!(shared.lost.load(Ordering::Acquire), 0);
        assert!(handoff.refused.is_none(), "the kept buffer was reused");
        assert_eq!(shared.delivered.load(Ordering::Acquire), 4 * 480);
    }

    #[test]
    fn each_buffer_carries_its_stamp_where_it_starts_on_the_takes_timeline() {
        let shared = Arc::new(Shared::default());
        let (to_writer, from_device) = mpsc::sync_channel::<Block>(8);
        let (_to_device, spare) = mpsc::sync_channel::<Vec<f32>>(1);
        let mut handoff = Handoff::new(to_writer, spare, 48_000, 1, Arc::clone(&shared));
        let buffer = [0.0f32; 480]; // 10 ms, mono
        handoff.hand_off(buffer.iter().copied(), 5 * TEN_MS);
        handoff.hand_off(buffer.iter().copied(), 6 * TEN_MS);
        // The device leaves 20 ms out: the last buffer starts after the gap, as the take does.
        handoff.hand_off(buffer.iter().copied(), 9 * TEN_MS);
        let stamps: Vec<Stamp> = from_device.try_iter().map(|b| b.stamp).collect();
        let at = |frame, nanos| Stamp { frame, nanos };
        assert_eq!(
            stamps,
            [
                at(0, 5 * TEN_MS),
                at(480, 6 * TEN_MS),
                at(480 + 480 + 960, 9 * TEN_MS)
            ]
        );
    }

    /// Two seconds of 512-frame buffers at 48 kHz from `start` nanoseconds, on the host's clock.
    fn line_from(start: u128) -> Line {
        let stamps: Vec<Stamp> = (0..=187)
            .map(|k| Stamp {
                frame: k * 512,
                nanos: start + u128::from(k * 512) * 1_000_000_000 / 48_000,
            })
            .collect();
        Line::fit(&stamps).expect("a line")
    }

    /// The click's output 5 ms behind the input on one clock, bar 1 at its frame 4800, its last
    /// buffer ending at `end`.
    fn heard(end: u64) -> Heard {
        Heard {
            name: "out".to_owned(),
            rate: 48_000,
            ticks: vec![4800],
            bar_one: 4800,
            played: Arc::new(Played::default()),
            line: Some(line_from(5_000_000)),
            end,
        }
    }

    #[test]
    fn a_take_that_heard_bar_one_is_placed_by_the_click_to_the_frame() {
        let input = Some(line_from(0));
        let placed = placement(96_000, 48_000, input, &heard(96_512), None, false).expect("placed");
        // Bar 1 played 100 ms + 5 ms after the take's first frame: 5040 frames.
        assert_eq!(placed.by, duet_schemas::song::By::Click);
        assert_eq!(placed.bar, Position::bar(1));
        assert!(
            (placed.at_secs * 48_000.0 - 5040.0).abs() < 1e-9,
            "{placed:?}"
        );
        assert!(placed.clock_ppm.expect("measured").abs() < 1e-6);
        let unreconciled = placement(96_000, 48_000, input, &heard(96_512), None, true).unwrap();
        assert_eq!(unreconciled, Placed::click(Position::bar(1), 0.105, None));
    }

    /// Ruling 18: a pair's calibration moves `at_secs` by its offset, to a whole frame.
    #[test]
    fn a_calibration_moves_at_secs_by_its_offset_to_a_whole_frame() {
        let input = Some(line_from(0));
        // The timestamps put bar 1 at 5040 frames (0.105 s); the pair is heard 8.01 ms earlier:
        // 4655.52 frames, to the frame 4656.
        let placed =
            placement(96_000, 48_000, input, &heard(96_512), Some(-8.01), false).expect("placed");
        let frames = placed.at_secs * 48_000.0;
        assert!((frames - 4656.0).abs() < 1e-9, "{placed:?}");
        assert_eq!(placed.at_secs, 4656.0 / 48_000.0);
        // A calibration that would put bar 1 before the take's first frame places nothing.
        assert_eq!(
            placement(96_000, 48_000, input, &heard(96_512), Some(-200.0), false),
            None
        );
    }

    /// The plan's `round((reported_secs + offset_ms / 1000) × rate)`: one rounding, not two.
    #[test]
    fn the_calibrated_offset_is_rounded_once() {
        let input = Some(line_from(0));
        // Bar 1 played 5040.48 frames after the take's first frame; the pair is heard 0.04 of a
        // frame later. Rounded once that is 5040.52, 5041; rounded twice it would be 5040.
        let late = Heard {
            line: Some(line_from(5_010_000)),
            ..heard(96_512)
        };
        let placed = placement(96_000, 48_000, input, &late, Some(0.04 / 48.0), false).unwrap();
        assert!(
            (placed.at_secs * 48_000.0 - 5041.0).abs() < 1e-6,
            "{placed:?}"
        );
    }

    /// Review Focus 5: a take stopped during the count-in, before bar 1 sounds, is not placed.
    #[test]
    fn a_take_that_ended_before_bar_one_is_not_placed() {
        let input = Some(line_from(0));
        // The click never got to bar 1's frame.
        assert_eq!(
            placement(96_000, 48_000, input, &heard(4800), None, false),
            None
        );
        // The click did, after the take's last frame.
        assert_eq!(
            placement(5000, 48_000, input, &heard(96_512), None, false),
            None
        );
        // No click played at all.
        let silent = Heard {
            line: None,
            end: 0,
            ..heard(96_512)
        };
        assert_eq!(placement(96_000, 48_000, input, &silent, None, false), None);
    }

    #[test]
    fn lost_input_is_written_as_silence_where_it_was() {
        let path = scratch("lost.flac");
        let mut writer = TakeWriter::create(&path, 48_000, 1, Depth::Sixteen).expect("create");
        let (spare, _) = mpsc::sync_channel::<Vec<f32>>(4);
        let mut take = Take::new(None);
        let block = |lost_before, value: f32| Block {
            lost_before,
            samples: vec![value; 4],
            stamp: Stamp { frame: 0, nanos: 0 },
        };
        take.add(&mut writer, block(0, 0.5), &spare).expect("add");
        take.add(&mut writer, block(3, 0.25), &spare).expect("add");
        assert_eq!(take.losses, [Loss { at: 4, frames: 3 }]);
        writer.finish().expect("finish");
        let mut expected = vec![0.5; 4];
        expected.extend([0.0; 3]);
        expected.extend([0.25; 4]);
        assert_eq!(read_back(&path), expected);
    }

    /// `--seconds` ends the take at its frame, silence and losses included: input lost past
    /// the limit was never part of the take, and is not reported as written.
    #[test]
    fn the_limit_cuts_the_take_and_its_losses() {
        let path = scratch("limit.flac");
        let mut writer = TakeWriter::create(&path, 48_000, 1, Depth::Sixteen).expect("create");
        let (spare, _) = mpsc::sync_channel::<Vec<f32>>(4);
        let mut take = Take::new(Some(10));
        let block = |lost_before| Block {
            lost_before,
            samples: vec![0.5; 4],
            stamp: Stamp { frame: 0, nanos: 0 },
        };
        take.add(&mut writer, block(0), &spare).expect("add");
        // 4 written; 3 lost then 4 more fill 7 of the 6 left: cut at 10.
        take.add(&mut writer, block(3), &spare).expect("add");
        assert!(take.full);
        // Past the limit: nothing is written or reported.
        take.add(&mut writer, block(5), &spare).expect("add");
        assert_eq!(take.losses, [Loss { at: 4, frames: 3 }]);
        assert_eq!(writer.frames(), 10);
        let written = writer.finish().expect("finish");
        assert_eq!(written.frames, 10);

        // A loss larger than the room left is reported as the silence written, not the loss.
        let path = scratch("limit-loss.flac");
        let mut writer = TakeWriter::create(&path, 48_000, 1, Depth::Sixteen).expect("create");
        let mut take = Take::new(Some(10));
        take.add(&mut writer, block(0), &spare).expect("add");
        take.add(&mut writer, block(8), &spare).expect("add");
        assert!(take.full);
        assert_eq!(take.losses, [Loss { at: 4, frames: 6 }]);
        assert_eq!(writer.finish().expect("finish").frames, 10);
    }

    #[test]
    fn a_device_is_picked_by_its_name_or_a_part_only_it_holds() {
        let inputs = names(&["MacBook Pro Microphone", "Scarlett 2i2 USB", "USB Mic"]);
        assert_eq!(pick_device(&inputs, "Scarlett 2i2 USB").unwrap(), 1);
        assert_eq!(pick_device(&inputs, "scarlett").unwrap(), 1);
        assert_eq!(pick_device(&inputs, "USB Mic").unwrap(), 2);
        let several = pick_device(&inputs, "usb").unwrap_err().to_string();
        assert!(several.contains("more than one"), "{several}");
        let none = pick_device(&inputs, "Yeti").unwrap_err().to_string();
        assert!(none.contains("MacBook Pro Microphone"), "{none}");
        assert!(pick_device(&inputs, " ").is_err());
    }

    #[test]
    fn positions_and_lengths_read_as_time() {
        assert_eq!(at(0, 48_000), "0:00.0");
        assert_eq!(at(48_000 * 75 + 4_800, 48_000), "1:15.1");
        assert_eq!(span(480, 48_000), "10 ms");
        assert_eq!(span(10, 48_000), "under 1 ms");
        assert_eq!(span(0, 48_000), "0 ms");
    }

    #[test]
    fn only_a_new_flac_path_in_a_real_directory_is_recorded_to() {
        assert!(refuse_output(Path::new("take.wav")).is_err());
        assert!(refuse_output(Path::new("take")).is_err());
        let missing = refuse_output(Path::new("/nonexistent-dir/new-take.flac"))
            .unwrap_err()
            .to_string();
        assert!(missing.contains("not a directory"), "{missing}");
        assert!(refuse_output(Path::new("new-take.FLAC")).is_ok());
        assert!(refuse_output(&std::env::temp_dir().join("duet-never-made.flac")).is_ok());
    }
}
