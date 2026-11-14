//! `duet play`: sound a song through the built-in SoundFont synth, or send it to a MIDI
//! port.
//!
//! The events, their times and the synth's rendering are `duet-play`'s, and tested there.
//! What this module adds, opening the audio device and the port and keeping time against
//! the wall clock, runs only on a machine with a device: no test in CI covers it.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use clap::Args;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};
use duet_play::{Renderer, Schedule, load_soundfont, send_timed};
use etcetera::{AppStrategy, AppStrategyArgs, choose_app_strategy};

use crate::config::Config;
use crate::io::{Format, read, song_format};

/// The environment variable naming the directory duet's installed files are in, such as
/// its SoundFont, overriding the platform's data directory.
pub const DATA_DIR_VAR: &str = "DUET_DATA_DIR";

/// The installed SoundFont's file name, in the data directory or in `share/duet` beside
/// the binary.
pub const SOUNDFONT_NAME: &str = "default.sf2";

#[derive(Args)]
pub struct PlayArgs {
    /// The song to play (MIDI, notefmt text, JSON or TOML), or - for standard input.
    input: PathBuf,
    #[arg(long, value_enum)]
    from: Option<Format>,
    /// The SoundFont (.sf2) to sound the song with, instead of duet.toml's `[play]`
    /// `soundfont` or the one duet installs.
    #[arg(long, conflicts_with = "port")]
    soundfont: Option<PathBuf>,
    /// Send the song to this MIDI port instead, such as a DAW's or a synth's: its name, or
    /// a part of it only one port's name holds. No SoundFont is needed.
    #[arg(long)]
    port: Option<String>,
}

pub fn run(args: PlayArgs, config: &Config) -> Result<()> {
    let format = song_format(&args.input, args.from, Format::Text, "duet play reads")?;
    let song = read(&args.input, format)?;
    let schedule = Schedule::from_song(&song)?;
    match args.port {
        Some(port) => to_port(&schedule, &port, &args.input),
        None => {
            let font = find_soundfont(args.soundfont, config)?;
            through_synth(&schedule, &font, &args.input)
        }
    }
}

/// Where the installed SoundFont is looked for: the data directory (`DUET_DATA_DIR`, or
/// the platform's), then `share/duet` beside the binary's directory, where a package
/// manager such as Homebrew puts it.
pub fn installed_soundfonts() -> Vec<PathBuf> {
    let mut out = Vec::new();
    match std::env::var_os(DATA_DIR_VAR).filter(|v| !v.is_empty()) {
        Some(dir) => out.push(PathBuf::from(dir).join(SOUNDFONT_NAME)),
        None => {
            if let Ok(strategy) = choose_app_strategy(AppStrategyArgs {
                top_level_domain: String::new(),
                author: String::new(),
                app_name: "duet".to_owned(),
            }) {
                out.push(strategy.data_dir().join(SOUNDFONT_NAME));
            }
        }
    }
    if let Some(bin_dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
        && let Some(prefix) = bin_dir.parent()
    {
        out.push(prefix.join("share/duet").join(SOUNDFONT_NAME));
    }
    out
}

/// The SoundFont to use: `--soundfont`, then duet.toml's, then the installed one. One
/// that is named but missing is an error, never passed over for the next.
fn find_soundfont(given: Option<PathBuf>, config: &Config) -> Result<PathBuf> {
    if let Some(path) = given {
        return exists(path, "--soundfont");
    }
    if let Some((path, from)) = &config.soundfont {
        return exists(path.clone(), &format!("[play] soundfont in {from}"));
    }
    let looked = installed_soundfonts();
    if let Some(found) = looked.iter().find(|p| p.is_file()) {
        return Ok(found.clone());
    }
    let places: Vec<String> = looked
        .iter()
        .map(|p| format!("  {}", p.display()))
        .collect();
    bail!(
        "no SoundFont to play with: duet looked for its installed one at\n{}\n\
         Install duet with its SoundFont, or name a General MIDI .sf2 with --soundfont or \
         [play] soundfont in duet.toml. Or send the song to a DAW or synth with --port <name>.",
        places.join("\n")
    )
}

fn exists(path: PathBuf, what: &str) -> Result<PathBuf> {
    if path.is_file() {
        Ok(path)
    } else {
        bail!("{} ({what}) is not a file", path.display())
    }
}

fn length(d: Duration) -> String {
    let secs = d.as_secs();
    format!("{}:{:02}", secs / 60, secs % 60)
}

/// Sends the schedule to a MIDI port, keeping time with the wall clock.
fn to_port(schedule: &Schedule, wanted: &str, input: &Path) -> Result<()> {
    let midi = midir::MidiOutput::new("duet").context("opening MIDI output")?;
    let ports: Vec<(String, midir::MidiOutputPort)> = midi
        .ports()
        .into_iter()
        .filter_map(|p| midi.port_name(&p).ok().map(|n| (n, p)))
        .collect();
    let names = || -> String {
        if ports.is_empty() {
            "  (none: a port appears while the app that owns it, such as a DAW, is running)"
                .to_owned()
        } else {
            ports
                .iter()
                .map(|(n, _)| format!("  {n}"))
                .collect::<Vec<_>>()
                .join("\n")
        }
    };
    let lower = wanted.to_lowercase();
    let matching: Vec<&(String, midir::MidiOutputPort)> =
        match ports.iter().find(|(n, _)| n == wanted) {
            Some(exact) => vec![exact],
            None => ports
                .iter()
                .filter(|(n, _)| n.to_lowercase().contains(&lower))
                .collect(),
        };
    let (name, port) = match matching.as_slice() {
        [one] => *one,
        [] => bail!(
            "no MIDI port is named {wanted:?}; the ports are:\n{}",
            names()
        ),
        _ => bail!(
            "{wanted:?} is part of more than one MIDI port's name; name one:\n{}",
            matching
                .iter()
                .map(|(n, _)| format!("  {n}"))
                .collect::<Vec<_>>()
                .join("\n")
        ),
    };
    let mut out = midi
        .connect(port, "duet play")
        .map_err(|e| anyhow!("connecting to MIDI port {name:?}: {e}"))?;
    eprintln!(
        "playing {} ({}) to MIDI port {name}",
        input.display(),
        length(schedule.end())
    );
    let start = Instant::now();
    send_timed(
        schedule,
        || start.elapsed(),
        std::thread::sleep,
        |bytes| out.send(bytes),
    )
    .map_err(|e| anyhow!("sending to MIDI port {name:?}: {e}"))?;
    out.close();
    Ok(())
}

/// Sounds the schedule through the default audio output with a SoundFont synth, until
/// the song and its tail have played.
fn through_synth(schedule: &Schedule, font_path: &Path, input: &Path) -> Result<()> {
    let bytes =
        std::fs::read(font_path).with_context(|| format!("reading {}", font_path.display()))?;
    let font = load_soundfont(&bytes).with_context(|| font_path.display().to_string())?;
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| anyhow!("no audio output device; send the song to one with --port"))?;
    let supported = device
        .default_output_config()
        .context("reading the audio device's output format")?;
    let config = supported.config();
    let renderer = Renderer::new(schedule, &font, config.sample_rate)?;
    let done = Arc::new(AtomicBool::new(false));
    let failed: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));

    let stream = match supported.sample_format() {
        SampleFormat::F32 => stream::<f32>(&device, config, renderer, &done, &failed),
        SampleFormat::I16 => stream::<i16>(&device, config, renderer, &done, &failed),
        SampleFormat::U16 => stream::<u16>(&device, config, renderer, &done, &failed),
        SampleFormat::I32 => stream::<i32>(&device, config, renderer, &done, &failed),
        other => bail!("the audio device's sample format, {other}, is not one duet plays"),
    }?;
    stream.play().context("starting audio output")?;
    eprintln!(
        "playing {} ({}) with {}",
        input.display(),
        length(schedule.end()),
        font_path.display()
    );
    // The render's length, with room for the device's buffering, bounds the wait.
    let limit = schedule.end() + duet_play::TAIL + Duration::from_secs(5);
    let start = Instant::now();
    while !done.load(Ordering::Acquire) && start.elapsed() < limit {
        if let Some(e) = failed.lock().ok().and_then(|f| f.clone()) {
            bail!("audio output failed: {e}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    // Let the device play out what it has buffered.
    std::thread::sleep(Duration::from_millis(200));
    drop(stream);
    Ok(())
}

/// An output stream that fills each buffer from the renderer: left and right in the
/// first two channels, both mixed in a mono one, and silence in any more.
fn stream<T: SizedSample + FromSample<f32>>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    mut renderer: Renderer,
    done: &Arc<AtomicBool>,
    failed: &Arc<Mutex<Option<String>>>,
) -> Result<cpal::Stream> {
    let channels = usize::from(config.channels);
    let (done, failed) = (Arc::clone(done), Arc::clone(failed));
    let (mut left, mut right) = (Vec::new(), Vec::new());
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _| {
                let frames = data.len() / channels.max(1);
                left.resize(frames, 0.0);
                right.resize(frames, 0.0);
                renderer.fill(&mut left, &mut right);
                for (i, frame) in data.chunks_mut(channels.max(1)).enumerate() {
                    for (c, sample) in frame.iter_mut().enumerate() {
                        let v = match (channels, c) {
                            (1, _) => (left[i] + right[i]) / 2.0,
                            (_, 0) => left[i],
                            (_, 1) => right[i],
                            _ => 0.0,
                        };
                        *sample = T::from_sample(v);
                    }
                }
                if renderer.is_done() {
                    done.store(true, Ordering::Release);
                }
            },
            move |e| {
                if let Ok(mut f) = failed.lock() {
                    *f = Some(e.to_string());
                }
            },
            None,
        )
        .context("opening the audio output")
}
