//! Parse a MIDI file (via midly) into internal representation.

use anyhow::{Context, Result};
use midly::{MetaMessage, MidiMessage, Smf, TrackEventKind};

use super::event::{MidiData, MidiEvent, NoteRect, TimedMidiEvent, TrackInfo};
use super::tempo_map::TempoMap;

pub fn parse_midi(bytes: &[u8]) -> Result<(MidiData, TempoMap)> {
    let smf = Smf::parse(bytes).context("Failed to parse MIDI file")?;

    let ticks_per_quarter = match smf.header.timing {
        midly::Timing::Metrical(tpq) => tpq.as_int(),
        midly::Timing::Timecode(_, _) => {
            anyhow::bail!("SMPTE timecode not supported");
        }
    };

    let format = match smf.header.format {
        midly::Format::SingleTrack => 0u8,
        midly::Format::Parallel => 1,
        midly::Format::Sequential => 2,
    };

    let mut all_events = Vec::new();
    let mut all_note_rects = Vec::new();
    let mut track_infos = Vec::new();
    let mut tempo_changes: Vec<(u64, u32)> = Vec::new();
    let mut used_channels: u64 = 0;
    let mut max_port: u8 = 0;

    for (track_idx, track) in smf.tracks.iter().enumerate() {
        let mut abs_tick: u64 = 0;
        let mut track_name = String::new();
        let mut note_count: u32 = 0;
        let mut channel_counts = [0u32; 16];
        let mut program_by_channel = [None::<u8>; 16];
        let mut current_port: u8 = 0;
        let mut track_port: u8 = 0;
        let mut port_set = false;

        // Active notes: (key, channel, port) -> (start_tick, velocity)
        let mut active_notes: std::collections::HashMap<(u8, u8, u8), (u64, u8)> =
            std::collections::HashMap::new();

        for event in track {
            abs_tick += event.delta.as_int() as u64;

            match event.kind {
                TrackEventKind::Midi { channel, message } => {
                    let ch = channel.as_int();
                    let port = current_port;
                    match message {
                        MidiMessage::NoteOn { key, vel } => {
                            let k = key.as_int();
                            let v = vel.as_int();
                            if v == 0 {
                                // Note-off via velocity 0
                                if let Some((start, velocity)) =
                                    active_notes.remove(&(k, ch, port))
                                {
                                    all_note_rects.push(NoteRect {
                                        key: k,
                                        channel: ch,
                                        port,
                                        start_tick: start,
                                        end_tick: abs_tick,
                                        velocity,
                                        track: track_idx,
                                    });
                                }
                                all_events.push(TimedMidiEvent {
                                    tick: abs_tick,
                                    event: MidiEvent::NoteOff { port, channel: ch, key: k },
                                    track: track_idx,
                                });
                            } else {
                                // Close any existing note on same key/channel/port
                                if let Some((start, velocity)) =
                                    active_notes.remove(&(k, ch, port))
                                {
                                    all_note_rects.push(NoteRect {
                                        key: k,
                                        channel: ch,
                                        port,
                                        start_tick: start,
                                        end_tick: abs_tick,
                                        velocity,
                                        track: track_idx,
                                    });
                                }
                                active_notes.insert((k, ch, port), (abs_tick, v));
                                note_count += 1;
                                channel_counts[ch as usize] += 1;
                                used_channels |= 1u64 << (port as u64 * 16 + ch as u64);
                                all_events.push(TimedMidiEvent {
                                    tick: abs_tick,
                                    event: MidiEvent::NoteOn {
                                        port,
                                        channel: ch,
                                        key: k,
                                        vel: v,
                                    },
                                    track: track_idx,
                                });
                            }
                        }
                        MidiMessage::NoteOff { key, vel: _ } => {
                            let k = key.as_int();
                            if let Some((start, velocity)) = active_notes.remove(&(k, ch, port))
                            {
                                all_note_rects.push(NoteRect {
                                    key: k,
                                    channel: ch,
                                    port,
                                    start_tick: start,
                                    end_tick: abs_tick,
                                    velocity,
                                    track: track_idx,
                                });
                            }
                            all_events.push(TimedMidiEvent {
                                tick: abs_tick,
                                event: MidiEvent::NoteOff { port, channel: ch, key: k },
                                track: track_idx,
                            });
                        }
                        MidiMessage::ProgramChange { program } => {
                            let p = program.as_int();
                            program_by_channel[ch as usize] = Some(p);
                            all_events.push(TimedMidiEvent {
                                tick: abs_tick,
                                event: MidiEvent::ProgramChange {
                                    port,
                                    channel: ch,
                                    program: p,
                                },
                                track: track_idx,
                            });
                        }
                        MidiMessage::Controller { controller, value } => {
                            all_events.push(TimedMidiEvent {
                                tick: abs_tick,
                                event: MidiEvent::ControlChange {
                                    port,
                                    channel: ch,
                                    controller: controller.as_int(),
                                    value: value.as_int(),
                                },
                                track: track_idx,
                            });
                        }
                        MidiMessage::PitchBend { bend } => {
                            all_events.push(TimedMidiEvent {
                                tick: abs_tick,
                                event: MidiEvent::PitchBend {
                                    port,
                                    channel: ch,
                                    value: bend.as_int(),
                                },
                                track: track_idx,
                            });
                        }
                        MidiMessage::Aftertouch { key, vel } => {
                            all_events.push(TimedMidiEvent {
                                tick: abs_tick,
                                event: MidiEvent::PolyAftertouch {
                                    port,
                                    channel: ch,
                                    key: key.as_int(),
                                    pressure: vel.as_int(),
                                },
                                track: track_idx,
                            });
                        }
                        MidiMessage::ChannelAftertouch { vel } => {
                            all_events.push(TimedMidiEvent {
                                tick: abs_tick,
                                event: MidiEvent::ChannelAftertouch {
                                    port,
                                    channel: ch,
                                    pressure: vel.as_int(),
                                },
                                track: track_idx,
                            });
                        }
                    }
                }
                TrackEventKind::Meta(meta) => match meta {
                    MetaMessage::TrackName(name_bytes) => {
                        if let Ok(s) = std::str::from_utf8(name_bytes) {
                            track_name = s.trim().to_string();
                        }
                    }
                    MetaMessage::Tempo(tempo) => {
                        let us_per_q = tempo.as_int();
                        tempo_changes.push((abs_tick, us_per_q));
                        all_events.push(TimedMidiEvent {
                            tick: abs_tick,
                            event: MidiEvent::TempoChange(us_per_q),
                            track: track_idx,
                        });
                    }
                    MetaMessage::TimeSignature(num, den, _, _) => {
                        all_events.push(TimedMidiEvent {
                            tick: abs_tick,
                            event: MidiEvent::TimeSignature {
                                numerator: num,
                                denominator: den,
                            },
                            track: track_idx,
                        });
                    }
                    MetaMessage::MidiPort(port_u7) => {
                        current_port = port_u7.as_int().min(3);
                        if !port_set {
                            track_port = current_port;
                            port_set = true;
                        }
                        if current_port > max_port {
                            max_port = current_port;
                        }
                    }
                    _ => {}
                },
                TrackEventKind::SysEx(data) => {
                    all_events.push(TimedMidiEvent {
                        tick: abs_tick,
                        event: MidiEvent::SysEx(data.to_vec()),
                        track: track_idx,
                    });
                }
                _ => {}
            }
        }

        // Close any remaining active notes at the track end
        for ((k, ch, port), (start, velocity)) in active_notes.drain() {
            all_note_rects.push(NoteRect {
                key: k,
                channel: ch,
                port,
                start_tick: start,
                end_tick: abs_tick,
                velocity,
                track: track_idx,
            });
        }

        // Determine primary channel
        let primary_channel = channel_counts
            .iter()
            .enumerate()
            .max_by_key(|&(_, &count)| count)
            .and_then(|(ch, &count)| if count > 0 { Some(ch as u8) } else { None });

        let program = primary_channel.and_then(|ch| program_by_channel[ch as usize]);

        track_infos.push(TrackInfo {
            index: track_idx,
            name: track_name,
            channel: primary_channel,
            port: track_port,
            program,
            note_count,
            channel_note_counts: channel_counts,
            channel_programs: program_by_channel,
        });
    }

    // Sort events by tick (stable sort preserves track order within same tick)
    all_events.sort_by_key(|e| e.tick);

    // Sort note rects by start tick
    all_note_rects.sort_by_key(|n| n.start_tick);

    let total_ticks = all_events.last().map(|e| e.tick).unwrap_or(0);

    // Build tempo map
    tempo_changes.sort_by_key(|&(tick, _)| tick);
    tempo_changes.dedup_by_key(|e| e.0);
    let tempo_map = TempoMap::new(ticks_per_quarter, &tempo_changes);

    let port_count = (max_port + 1).max(1);

    let midi_data = MidiData {
        ticks_per_quarter,
        format,
        events: all_events,
        note_rects: all_note_rects,
        tracks: track_infos,
        total_ticks,
        used_channels,
        port_count,
    };

    log_info!(
        "Parse complete: format={}, tracks={}, events={}, notes={}, ticks={}, tpq={}, ports={}",
        format, midi_data.tracks.len(), midi_data.events.len(),
        midi_data.note_rects.len(), total_ticks, ticks_per_quarter, port_count
    );

    Ok((midi_data, tempo_map))
}
