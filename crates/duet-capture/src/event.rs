use serde::{Deserialize, Serialize};

/// The lane of a drum [`Event::Hit`].
///
/// `Asked` is the lane the user asked for, when the take is not told apart by sound.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
#[non_exhaustive]
pub enum HitLane {
    Kick,
    Snare,
    Hat,
    Asked,
}

/// What the analysis stages say about a take. Times are sample frames at the analysis rate.
///
/// This is the payload of `<stem>.capture.json`, and later of the MCP stream.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
#[non_exhaustive]
pub enum Event {
    /// Something started at `frame`, with the given strength.
    Onset { frame: u64, strength: f32 },
    /// A pitched note from `start` to `end`, as a MIDI key number.
    Note {
        start: u64,
        end: u64,
        midi: u8,
        confidence: f32,
        strength: f32,
    },
    /// A drum hit on a lane.
    Hit {
        frame: u64,
        lane: HitLane,
        strength: f32,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_round_trip_through_json() {
        let events = vec![
            Event::Onset {
                frame: 22_050,
                strength: 0.5,
            },
            Event::Note {
                start: 100,
                end: 4_100,
                midi: 60,
                confidence: 0.75,
                strength: 0.25,
            },
            Event::Hit {
                frame: 44_100,
                lane: HitLane::Snare,
                strength: 1.0,
            },
            Event::Hit {
                frame: 44_200,
                lane: HitLane::Asked,
                strength: 0.125,
            },
        ];
        let json = serde_json::to_string(&events).unwrap();
        let back: Vec<Event> = serde_json::from_str(&json).unwrap();
        assert_eq!(back, events);
        assert!(json.contains(r#""type":"onset""#), "{json}");
        assert!(json.contains(r#""lane":{"type":"snare"}"#), "{json}");
    }
}
