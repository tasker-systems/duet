//! `duet record`: a take from the machine's microphone, kept as a FLAC file that `duet capture`
//! reads like any other.
//!
//! Writing the file is `duet-capture`'s [`TakeWriter`], and tested there. What this module adds,
//! opening the input device and carrying its samples to the writer, runs only on a machine with
//! a device: no test in CI covers it. What it refuses before opening one is tested end to end.
//!
//! The device's callback never waits: it hands each buffer to the writing thread through a
//! bounded channel, and a buffer the channel cannot take is counted as lost. The writer puts
//! silence where it was, so what follows keeps its time, and says when and how much.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use clap::Args;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, SizedSample};
use duet_capture::{Depth, TakeWriter, Written};

/// Buffers waiting for the writer before the device's next one is counted as lost: at a
/// typical 512 frames a buffer, about seven seconds at 48 kHz.
const QUEUE: usize = 640;

#[derive(Args)]
pub struct RecordArgs {
    /// The FLAC file to write the take to. It must not exist: a take never replaces a file.
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

/// Input lost between two buffers the writer did take: `frames` of it, before the take's
/// frame `at`.
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
    /// Frames the device has delivered, lost ones included: when an overrun happened.
    delivered: AtomicU64,
    /// Frames lost since the last buffer the channel took.
    lost: AtomicU64,
    /// Where in the take the device reported overruns, in frames delivered.
    overruns: Mutex<Vec<u64>>,
    /// Why the stream ended, when the device ended it.
    failed: Mutex<Option<String>>,
}

pub fn run(args: RecordArgs) -> Result<()> {
    refuse_output(&args.output)?;
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

    let mut writer = TakeWriter::create(&args.output, rate, channels, depth)?;
    let shared = Arc::new(Shared::default());
    let (to_writer, from_device) = mpsc::sync_channel::<Block>(QUEUE);
    // Buffers handed back for reuse, so the callback rarely allocates. One the device has no
    // room for is dropped.
    let (to_device, spare) = mpsc::sync_channel::<Vec<f32>>(QUEUE);
    let stream = match supported.sample_format() {
        SampleFormat::F32 => input::<f32>(&device, config, to_writer, spare, &shared),
        SampleFormat::I16 => input::<i16>(&device, config, to_writer, spare, &shared),
        SampleFormat::U16 => input::<u16>(&device, config, to_writer, spare, &shared),
        SampleFormat::I32 => input::<i32>(&device, config, to_writer, spare, &shared),
        other => bail!("{name}'s sample format, {other}, is not one duet records"),
    }
    .with_context(|| format!("opening {name} for input"));
    let stream = match stream {
        Ok(stream) => stream,
        Err(e) => {
            // Nothing was recorded: the empty file is not a take.
            drop(writer);
            let _ = std::fs::remove_file(&args.output);
            return Err(e);
        }
    };

    let stop = Arc::new(AtomicBool::new(false));
    {
        let stop = Arc::clone(&stop);
        ctrlc::set_handler(move || stop.store(true, Ordering::Release))
            .context("catching Ctrl-C")?;
    }
    if let Err(e) = stream.play() {
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
    let mut take = Take::default();
    let ended = loop {
        if stop.load(Ordering::Acquire) {
            break None;
        }
        if let Some(e) = shared.failed.lock().ok().and_then(|f| f.clone()) {
            break Some(e);
        }
        match from_device.recv_timeout(Duration::from_millis(20)) {
            Ok(block) => {
                let samples = take.add(&mut writer, block, limit, &to_device)?;
                if samples.is_none() {
                    break None;
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break Some("the input stream closed".into()),
        }
    };
    drop(stream);
    // What the device delivered before the stream stopped is part of the take.
    while let Ok(block) = from_device.try_recv() {
        if take.add(&mut writer, block, limit, &to_device)?.is_none() {
            break;
        }
    }
    let lost_at_end = shared.lost.load(Ordering::Acquire);
    let written = writer.finish()?;
    let overruns = shared
        .overruns
        .lock()
        .map(|o| o.clone())
        .unwrap_or_default();
    report(&written, &take, lost_at_end, &overruns);
    match ended {
        None => Ok(()),
        Some(why) => bail!(
            "recording stopped: {why}; the take up to there is kept in {}",
            written.path.display()
        ),
    }
}

/// What the writing thread has seen of the take.
#[derive(Default)]
struct Take {
    losses: Vec<Loss>,
    /// The loudest sample, to tell a silent take from a quiet one.
    peak: f32,
}

impl Take {
    /// Writes one buffer, after silence for any input lost before it, up to `limit` frames.
    /// `None` once the limit is reached; the buffer is handed back for reuse.
    fn add(
        &mut self,
        writer: &mut TakeWriter,
        block: Block,
        limit: Option<u64>,
        spare: &SyncSender<Vec<f32>>,
    ) -> Result<Option<()>> {
        let channels = usize::from(writer.channels());
        let room =
            |writer: &TakeWriter| limit.map_or(u64::MAX, |l| l.saturating_sub(writer.frames()));
        if block.lost_before > 0 {
            let frames = block.lost_before.min(room(writer));
            self.losses.push(Loss {
                at: writer.frames(),
                frames: block.lost_before,
            });
            writer.silence(frames)?;
        }
        let frames = (block.samples.len() / channels) as u64;
        let keep = frames.min(room(writer)) as usize * channels;
        let samples = &block.samples[..keep];
        self.peak = samples.iter().fold(self.peak, |p, s| p.max(s.abs()));
        writer.write(samples)?;
        let _ = spare.try_send(block.samples);
        Ok(if room(writer) == 0 { None } else { Some(()) })
    }
}

/// Says what was written, and everything about the take the owner should know: input lost,
/// overruns, samples at full scale, and a take that is silent throughout.
fn report(written: &Written, take: &Take, lost_at_end: u64, overruns: &[u64]) {
    let rate = written.rate;
    eprintln!(
        "wrote {} ({})",
        written.path.display(),
        at(written.frames, rate)
    );
    for loss in &take.losses {
        eprintln!(
            "lost {} of input at {}, written as silence so what follows keeps its time",
            span(loss.frames, rate),
            at(loss.at, rate)
        );
    }
    if lost_at_end > 0 {
        eprintln!(
            "lost the last {} of input before recording stopped",
            span(lost_at_end, rate)
        );
    }
    for &frame in overruns {
        eprintln!(
            "the device reported an overrun at {}: some input may be missing there",
            at(frame, rate)
        );
    }
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

/// The input stream: each buffer converted to `f32` and handed to the writer without
/// waiting, or counted as lost.
fn input<T: SizedSample>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    to_writer: SyncSender<Block>,
    spare: Receiver<Vec<f32>>,
    shared: &Arc<Shared>,
) -> Result<cpal::Stream>
where
    f32: cpal::FromSample<T>,
{
    let channels = u64::from(config.channels.max(1));
    let (data, errors) = (Arc::clone(shared), Arc::clone(shared));
    let mut warned = false;
    device
        .build_input_stream(
            config,
            move |input: &[T], _| {
                let frames = input.len() as u64 / channels;
                data.delivered.fetch_add(frames, Ordering::AcqRel);
                let mut samples = spare.try_recv().unwrap_or_default();
                samples.clear();
                samples.extend(input.iter().map(|&s| cpal::Sample::to_sample::<f32>(s)));
                let lost_before = data.lost.swap(0, Ordering::AcqRel);
                match to_writer.try_send(Block {
                    lost_before,
                    samples,
                }) {
                    Ok(()) => {}
                    Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
                        data.lost.fetch_add(lost_before + frames, Ordering::AcqRel);
                    }
                }
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

/// Refuses an output that exists or is not named `.flac`, before any device is opened.
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

/// A position in the take, as minutes and seconds to a tenth.
fn at(frames: u64, rate: u32) -> String {
    let tenths = (frames as f64 / f64::from(rate) * 10.0).round() as u64;
    format!("{}:{:02}.{}", tenths / 600, tenths / 10 % 60, tenths % 10)
}

/// A length of input, in milliseconds.
fn span(frames: u64, rate: u32) -> String {
    format!("{:.0} ms", frames as f64 * 1000.0 / f64::from(rate))
}

#[cfg(test)]
mod tests {
    use super::{at, pick_device, refuse_output, span};

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|&n| n.to_owned()).collect()
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
    }

    #[test]
    fn only_a_new_flac_path_is_recorded_to() {
        assert!(refuse_output(std::path::Path::new("take.wav")).is_err());
        assert!(refuse_output(std::path::Path::new("take")).is_err());
        assert!(refuse_output(std::path::Path::new("/nonexistent-dir/new-take.FLAC")).is_ok());
    }
}
