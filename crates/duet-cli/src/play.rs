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
use crate::io::{Format, FormatParser, read, song_format};

/// The environment variable naming the directory duet's installed files are in, such as
/// its SoundFont, overriding the platform's data directory.
pub const DATA_DIR_VAR: &str = "DUET_DATA_DIR";

/// The installed SoundFont's file name, in the data directory or in `share/duet` beside
/// the binary.
pub const SOUNDFONT_NAME: &str = "default.sf2";

#[derive(Args)]
pub struct PlayArgs {
    /// The song to play: a .duet document, MIDI or ABC by its name, or - for standard input
    /// (a document if it starts with {, MIDI, or ABC with --from abc). A view's text plays
    /// through `duet apply`: `duet apply take.notes -o - | duet play -`.
    input: PathBuf,
    #[arg(long, value_parser = FormatParser)]
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
    let format = song_format(&args.input, args.from, "duet play reads")?;
    let song = read(&args.input, format, "duet play reads")?;
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

/// The port `wanted` names among `names`: the one with exactly that name, or else the one
/// whose name holds it, ignoring case. Refused when none or several do, or when it is
/// empty, so a port is never picked by accident.
pub fn pick_port(names: &[String], wanted: &str) -> Result<usize> {
    let list = |which: &mut dyn Iterator<Item = &String>| -> String {
        let lines: Vec<String> = which.map(|n| format!("  {n}")).collect();
        if lines.is_empty() {
            "  (none: a port appears while the app that owns it, such as a DAW, is running)"
                .to_owned()
        } else {
            lines.join("\n")
        }
    };
    if wanted.trim().is_empty() {
        bail!(
            "--port needs a port's name; the ports are:\n{}",
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
            "no MIDI port is named {wanted:?}; the ports are:\n{}",
            list(&mut names.iter())
        ),
        _ if found.iter().all(|&i| names[i] == wanted) => bail!(
            "{} MIDI ports are named {wanted:?}, and duet cannot tell them apart",
            found.len()
        ),
        _ => bail!(
            "{wanted:?} is part of more than one MIDI port's name; name one:\n{}",
            list(&mut found.iter().map(|&i| &names[i]))
        ),
    }
}

/// What a port must be sent when playing stops early, so nothing is left sounding: a
/// note-off for each key still held, then sustain off and all notes off on each channel
/// used.
#[derive(Default)]
pub struct PortCleanup {
    held: std::collections::BTreeMap<(u8, u8), u32>,
    channels: std::collections::BTreeSet<u8>,
}

impl PortCleanup {
    /// Notes a message as sent.
    pub fn sent(&mut self, bytes: &[u8]) {
        let (Some(&status), Some(&key)) = (bytes.first(), bytes.get(1)) else {
            return;
        };
        let channel = status & 0x0F;
        self.channels.insert(channel);
        let vel = bytes.get(2).copied().unwrap_or(0);
        match status & 0xF0 {
            0x90 if vel > 0 => *self.held.entry((channel, key)).or_default() += 1,
            0x80 | 0x90 => {
                self.held.remove(&(channel, key));
            }
            _ => {}
        }
    }

    /// The messages that silence the port.
    pub fn messages(&self) -> Vec<[u8; 3]> {
        let offs = self.held.keys().map(|&(ch, key)| [0x80 | ch, key, 64]);
        let resets = self
            .channels
            .iter()
            .flat_map(|&ch| [[0xB0 | ch, 64, 0], [0xB0 | ch, 123, 0]]);
        offs.chain(resets).collect()
    }
}

/// Why sending to a port stopped early.
enum Stop {
    Interrupted,
    Send(midir::SendError),
}

/// Sends the schedule to a MIDI port, keeping time with the wall clock. On Ctrl-C or a
/// failed send, the port is sent what silences it before duet stops.
fn to_port(schedule: &Schedule, wanted: &str, input: &Path) -> Result<()> {
    let midi = midir::MidiOutput::new("duet").context("opening MIDI output")?;
    let mut ports: Vec<(String, midir::MidiOutputPort)> = midi
        .ports()
        .into_iter()
        .filter_map(|p| midi.port_name(&p).ok().map(|n| (n, p)))
        .collect();
    let names: Vec<String> = ports.iter().map(|(n, _)| n.clone()).collect();
    let (name, port) = ports.swap_remove(pick_port(&names, wanted)?);
    let mut out = midi
        .connect(&port, "duet play")
        .map_err(|e| anyhow!("connecting to MIDI port {name:?}: {e}"))?;

    let stop = Arc::new(AtomicBool::new(false));
    {
        let stop = Arc::clone(&stop);
        ctrlc::set_handler(move || stop.store(true, Ordering::Release))
            .context("catching Ctrl-C")?;
    }
    eprintln!(
        "playing {} ({}) to MIDI port {name}; Ctrl-C stops it",
        input.display(),
        length(schedule.end())
    );
    let mut cleanup = PortCleanup::default();
    let start = Instant::now();
    let stopped = || stop.load(Ordering::Acquire);
    let result = send_timed(
        schedule,
        || start.elapsed(),
        // Sleep in short steps, so Ctrl-C is answered at once.
        |d| {
            let until = Instant::now() + d;
            while !stopped() {
                let left = until.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    break;
                }
                std::thread::sleep(left.min(Duration::from_millis(10)));
            }
        },
        |bytes| {
            if stopped() {
                return Err(Stop::Interrupted);
            }
            out.send(bytes).map_err(Stop::Send)?;
            cleanup.sent(bytes);
            Ok(())
        },
    );
    let result = match result {
        // The wait to the end was cut short: the song's events were all sent.
        Ok(()) if stopped() => Err(Stop::Interrupted),
        other => other,
    };
    match result {
        Ok(()) => {
            out.close();
            Ok(())
        }
        Err(stop) => {
            // Best effort: a port that refused one send may refuse these too.
            for message in cleanup.messages() {
                let _ = out.send(&message);
            }
            out.close();
            match stop {
                Stop::Interrupted => bail!("stopped; every note on {name} was turned off"),
                Stop::Send(e) => bail!(
                    "sending to MIDI port {name:?}: {e}; duet tried to turn off every note on it"
                ),
            }
        }
    }
}

/// Whether an audio stream error leaves the stream playing: an underrun, the default
/// output moving to another device, or real-time priority refused. cpal reports these
/// through the same callback as errors that end a stream.
fn stream_goes_on(kind: cpal::ErrorKind) -> bool {
    matches!(
        kind,
        cpal::ErrorKind::Xrun | cpal::ErrorKind::DeviceChanged | cpal::ErrorKind::RealtimeDenied
    )
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
    while !done.load(Ordering::Acquire) {
        if let Some(e) = failed.lock().ok().and_then(|f| f.clone()) {
            bail!("audio output failed: {e}");
        }
        if start.elapsed() >= limit {
            bail!(
                "the audio device stopped asking for sound before the song's end; it may \
                 have been disconnected"
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    // Let the device play out what it has buffered.
    std::thread::sleep(Duration::from_millis(200));
    drop(stream);
    Ok(())
}

/// An output stream that fills each buffer from the renderer: left and right in the
/// first two channels, both mixed in a mono one, and silence in any more. An error that
/// leaves the stream playing is said on standard error once; any other stops playing.
fn stream<T: SizedSample + FromSample<f32>>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    mut renderer: Renderer,
    done: &Arc<AtomicBool>,
    failed: &Arc<Mutex<Option<String>>>,
) -> Result<cpal::Stream> {
    let channels = usize::from(config.channels).max(1);
    let (done, failed) = (Arc::clone(done), Arc::clone(failed));
    // Sized for a large device buffer up front, so the callback rarely allocates.
    let (mut left, mut right) = (vec![0.0f32; 8192], vec![0.0f32; 8192]);
    let mut warned = false;
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _| {
                let frames = data.len() / channels;
                if frames > left.len() {
                    left.resize(frames, 0.0);
                    right.resize(frames, 0.0);
                }
                let (l, r) = (&mut left[..frames], &mut right[..frames]);
                renderer.fill(l, r);
                for (i, frame) in data.chunks_exact_mut(channels).enumerate() {
                    for (c, sample) in frame.iter_mut().enumerate() {
                        let v = match (channels, c) {
                            (1, _) => (l[i] + r[i]) / 2.0,
                            (_, 0) => l[i],
                            (_, 1) => r[i],
                            _ => 0.0,
                        };
                        *sample = T::from_sample(v);
                    }
                }
                // A partial frame at the end, which a device should never send, is silent.
                for sample in data.chunks_exact_mut(channels).into_remainder() {
                    *sample = T::from_sample(0.0);
                }
                if renderer.is_done() {
                    done.store(true, Ordering::Release);
                }
            },
            move |e| {
                if stream_goes_on(e.kind()) {
                    if !warned {
                        eprintln!("duet: audio glitch or device change, still playing: {e}");
                        warned = true;
                    }
                } else if let Ok(mut f) = failed.lock() {
                    *f = Some(e.to_string());
                }
            },
            None,
        )
        .context("opening the audio output")
}

#[cfg(test)]
mod tests {
    use super::{PortCleanup, pick_port};

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn a_port_is_picked_by_its_name_or_a_part_only_it_holds() {
        let ports = names(&[
            "IAC Driver Bus 1",
            "IAC Driver Bus 10",
            "Logic Pro Virtual In",
        ]);
        assert_eq!(pick_port(&ports, "IAC Driver Bus 1").expect("exact"), 0);
        assert_eq!(pick_port(&ports, "logic").expect("part, any case"), 2);
        assert_eq!(pick_port(&ports, "bus 10").expect("part"), 1);
    }

    #[test]
    fn a_port_name_that_is_not_one_ports_is_refused() {
        let ports = names(&["IAC Driver Bus 1", "IAC Driver Bus 2", "Twin", "Twin"]);
        let err = |wanted: &str| pick_port(&ports, wanted).expect_err(wanted).to_string();
        assert!(
            err("IAC").contains("part of more than one"),
            "{}",
            err("IAC")
        );
        assert!(err("IAC").contains("  IAC Driver Bus 2"), "{}", err("IAC"));
        assert!(err("nope").starts_with("no MIDI port is named \"nope\""));
        assert!(err("nope").contains("  Twin"), "lists the ports");
        assert!(
            err("").starts_with("--port needs a port's name"),
            "{}",
            err("")
        );
        assert!(err("  ").starts_with("--port needs a port's name"));
        assert!(
            err("Twin").contains("2 MIDI ports are named \"Twin\""),
            "{}",
            err("Twin")
        );
        let none = pick_port(&[], "x").expect_err("none").to_string();
        assert!(none.contains("(none: a port appears while"), "{none}");
    }

    #[test]
    fn stopping_early_turns_off_what_is_held_and_resets_each_channel() {
        let mut cleanup = PortCleanup::default();
        for message in [
            &[0xC0, 5][..],
            &[0xB0, 64, 127],
            &[0x90, 60, 80],
            &[0x90, 64, 80],
            &[0x80, 60, 64],
            &[0x92, 40, 90],
            &[0x92, 41, 0],
        ] {
            cleanup.sent(message);
        }
        assert_eq!(
            cleanup.messages(),
            [
                [0x80, 64, 64],
                [0x82, 40, 64],
                [0xB0, 64, 0],
                [0xB0, 123, 0],
                [0xB2, 64, 0],
                [0xB2, 123, 0],
            ]
        );
    }
}
