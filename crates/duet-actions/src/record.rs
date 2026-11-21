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

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::Context;
use crate::take::{at, span};
use anyhow::{Context as _, Result, anyhow, bail};
use clap::Args;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, SizedSample};
use duet_capture::{Depth, TakeWriter, Written};
use schemars::JsonSchema;

// `--song` uses the click and the clock arithmetic; until it is wired in, only tests do.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "`record --song` is wired to the click next")
)]
mod click;

/// Buffers waiting for the writer before the device's next one is counted as lost: at a
/// typical 512 frames a buffer, about seven seconds at 48 kHz.
const QUEUE: usize = 640;

#[derive(Debug, Args, JsonSchema)]
pub struct RecordArgs {
    /// The FLAC file to write the take to. It must not exist: a take never replaces a file. In
    /// a project, a take's name instead (lowercase words joined by hyphens, no .flac): the take
    /// is recorded into takes/TAKE/TAKE.flac and added to the project.
    output: PathBuf,
    /// The input device to record from: its name, or a part of it only one input's name
    /// holds. Without it, the system's default input.
    #[arg(long)]
    device: Option<String>,
    /// Stop after this many seconds, instead of at Ctrl-C.
    #[arg(long, value_parser = positive_seconds)]
    seconds: Option<f64>,
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

pub fn run(ctx: &Context, mut args: RecordArgs) -> Result<()> {
    let into = into_project(ctx, &args.output)?;
    if let Some((_, _, path)) = &into {
        args.output.clone_from(path);
    }
    if into.is_none() {
        refuse_output(&args.output)?;
    }
    let stop = Arc::new(AtomicBool::new(false));
    {
        let stop = Arc::clone(&stop);
        ctrlc::set_handler(move || stop.store(true, Ordering::Release))
            .context("catching Ctrl-C")?;
    }
    let host = cpal::default_host();
    let device = match &args.device {
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
    let config = supported.config();
    let depth = depth_of(supported.sample_format())?;
    let (rate, channels) = (config.sample_rate, config.channels);

    let shared = Arc::new(Shared::default());
    let (to_writer, from_device) = mpsc::sync_channel::<Block>(QUEUE);
    // Buffers handed back for reuse, so the callback rarely allocates. One the device has no
    // room for is dropped.
    let (to_device, spare) = mpsc::sync_channel::<Vec<f32>>(QUEUE);
    let handoff = Handoff::new(to_writer, spare, rate, channels, Arc::clone(&shared));
    // The stream is built before the file exists, so a device that refuses leaves nothing.
    let stream = match supported.sample_format() {
        SampleFormat::F32 => input::<f32>(&device, config, handoff, &shared),
        SampleFormat::I16 => input::<i16>(&device, config, handoff, &shared),
        SampleFormat::U16 => input::<u16>(&device, config, handoff, &shared),
        SampleFormat::I32 => input::<i32>(&device, config, handoff, &shared),
        other => bail!("{name}'s sample format, {other}, is not one duet records"),
    }
    .with_context(|| format!("opening {name} for input"))?;
    if let Some((_, _, path)) = &into
        && let Some(dir) = path.parent()
    {
        std::fs::create_dir_all(dir).with_context(|| dir.display().to_string())?;
    }
    let mut writer = TakeWriter::create(&args.output, rate, channels, depth)?;
    if let Err(e) = stream.play() {
        // Nothing was recorded: the empty file is not a take.
        drop(writer);
        let _ = std::fs::remove_file(&args.output);
        return Err(e).with_context(|| format!("starting input from {name}"));
    }
    let until = match args.seconds {
        Some(secs) => format!("for {secs} s"),
        None => "until Ctrl-C".to_owned(),
    };
    eprintln!(
        "recording {} from {name} ({rate} Hz, {channels} channel{}, {}-bit) {until}",
        args.output.display(),
        if channels == 1 { "" } else { "s" },
        depth.bits(),
    );

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
            return Err(e).context(kept(&args.output));
        }
    };
    drop(stream);
    // What the device delivered before the stream stopped is part of the take.
    let drained = (|| -> Result<()> {
        while let Ok(block) = from_device.try_recv() {
            take.add(&mut writer, block, &to_device)?;
        }
        Ok(())
    })();
    if let Err(e) = drained {
        take.report_losses(rate, &shared);
        return Err(e).context(kept(&args.output));
    }
    let written = match writer.finish() {
        Ok(written) => written,
        Err(e) => {
            take.report_losses(rate, &shared);
            return Err(e).context(kept(&args.output));
        }
    };
    eprintln!(
        "wrote {} ({})",
        written.path.display(),
        at(written.frames, rate)
    );
    take.report_losses(rate, &shared);
    report_level(&written, &take);
    // A take is no document, so this records nothing; it goes through the context as every
    // write does.
    ctx.wrote(&written.path, false)?;
    // In a project the take is added, whether the recording ran to its end or stopped: what
    // was written is kept either way.
    if let Some((project, name, _)) = &into {
        let take = crate::take::add_recorded(ctx, project, name, &written.path)?;
        eprintln!(
            "added take {name}: {}, in {}",
            take.digest,
            crate::PROJECT_FILE
        );
    }
    match ended {
        None => Ok(()),
        Some(why) => bail!("recording stopped: {why}; {}", kept(&written.path)),
    }
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
}

impl Take {
    fn new(limit: Option<u64>) -> Take {
        Take {
            limit,
            full: false,
            losses: Vec::new(),
            peak: 0.0,
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
        self.shared
            .delivered
            .fetch_add(gap + frames, Ordering::AcqRel);
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

/// Refuses an output that exists, is not named `.flac`, or is in a directory that does not
/// exist, before any device is opened.
/// Where `duet record <take>` records in a project: the project, the take's name, and
/// `takes/<take>/<take>.flac` under its root. `None` when the output is a file's path, as it
/// is outside a project. A name the project has a take by already is refused, as is a file
/// there already, before any device is opened.
fn into_project(
    ctx: &Context,
    output: &Path,
) -> Result<Option<(crate::Project, duet_schemas::project::Name, PathBuf)>> {
    let named = output.extension().is_none()
        && output.components().count() == 1
        && output
            .to_str()
            .is_some_and(|s| s.parse::<duet_schemas::project::Name>().is_ok());
    if !named {
        return Ok(None);
    }
    let name: duet_schemas::project::Name = output
        .to_str()
        .unwrap_or_default()
        .parse()
        .map_err(anyhow::Error::msg)?;
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

    use super::{Block, Clock, Handoff, Loss, Shared, Take, at, pick_device, refuse_output, span};

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
    fn lost_input_is_written_as_silence_where_it_was() {
        let path = scratch("lost.flac");
        let mut writer = TakeWriter::create(&path, 48_000, 1, Depth::Sixteen).expect("create");
        let (spare, _) = mpsc::sync_channel::<Vec<f32>>(4);
        let mut take = Take::new(None);
        let block = |lost_before, value: f32| Block {
            lost_before,
            samples: vec![value; 4],
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
