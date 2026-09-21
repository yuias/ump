//! Audio output via cpal.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::Stream;
use ump_playback::sequencer::Sequencer;
use ump_playback::synth::engine::SynthPool;

use crate::state::{SharedState, SharedStateSink};

pub struct AudioOutput {
    _stream: Stream,
}

/// Query the default output device and return its preferred sample rate.
pub fn query_sample_rate() -> Result<u32> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .context("No audio output device found")?;
    let default_config = device
        .default_output_config()
        .context("Failed to get default output config")?;
    let rate = default_config.sample_rate();
    log_info!("Audio device: {:?}, sample_rate={}", device.description().map(|d| d.name().to_owned()).unwrap_or_default(), rate);
    Ok(rate)
}

impl AudioOutput {
    /// Build and start the audio output stream.
    /// The stream runs the sequencer and synthesizer in the cpal callback.
    pub fn start(
        sequencer: Arc<Mutex<Sequencer>>,
        synth: Arc<Mutex<Option<SynthPool>>>,
        shared: Arc<SharedState>,
        sample_rate: u32,
    ) -> Result<Self> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .context("No audio output device found")?;

        let config = cpal::StreamConfig {
            channels: 2,
            sample_rate,
            buffer_size: cpal::BufferSize::Default,
        };

        let channels = config.channels as usize;

        let err_fn = |err: cpal::Error| {
            log_error!("Audio stream error: {}", err);
        };

        let stream = device.build_output_stream(
            config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                let sample_count = data.len() / channels;
                let mut left = vec![0.0f32; sample_count];
                let mut right = vec![0.0f32; sample_count];

                if let (Ok(mut seq), Ok(mut syn_opt)) =
                    (sequencer.try_lock(), synth.try_lock())
                    && let Some(ref mut syn) = *syn_opt
                {
                    render(&mut seq, syn, &mut left, &mut right, &shared);
                }

                // Interleave into output buffer
                for i in 0..sample_count {
                    for ch in 0..channels {
                        data[i * channels + ch] = if ch == 0 { left[i] } else { right[i] };
                    }
                }
            },
            err_fn,
            None,
        ).context("Failed to build audio stream")?;

        stream.play().context("Failed to start audio stream")?;

        Ok(AudioOutput { _stream: stream })
    }
}

/// Apply transport requests from the UI thread, then render one buffer.
/// `left`/`right` arrive zeroed and stay silent unless playback is active.
fn render(
    seq: &mut Sequencer,
    synth: &mut SynthPool,
    left: &mut [f32],
    right: &mut [f32],
    shared: &SharedState,
) {
    let mut sink = SharedStateSink(shared);

    seq.set_muted_channels(shared.muted_channels.load(Ordering::Relaxed));

    // seek_tick stores target + 1 so that 0 can mean "no request".
    let seek_raw = shared.seek_tick.swap(0, Ordering::Relaxed);
    if seek_raw > 0 {
        seq.seek_to_tick(seek_raw - 1, synth, &mut sink);
        shared.current_tick.store(seq.current_tick(), Ordering::Relaxed);
        shared.finished.store(false, Ordering::Relaxed);
        sync_drum_channels(synth, shared);
    }

    if shared.stopped.load(Ordering::Relaxed)
        || !shared.is_playing()
        || shared.finished.load(Ordering::Relaxed)
    {
        return;
    }

    // Read before rendering so a MasterVolume SysEx takes effect next buffer.
    let volume = shared.get_volume_f32();
    seq.fill_buffer(synth, left, right, &mut sink);
    for s in left.iter_mut().chain(right.iter_mut()) {
        *s *= volume;
    }

    shared.current_tick.store(seq.current_tick(), Ordering::Relaxed);
    sync_drum_channels(synth, shared);
    if seq.is_finished() {
        shared.finished.store(true, Ordering::Relaxed);
        shared.playing.store(false, Ordering::Relaxed);
    }
}

/// Mirror the synthesizer's drum map into `SharedState` for the UI.
///
/// The synthesizer is the only place that knows it, since a channel reaches the
/// drum bank through GS, XG and bank-select paths that ump does not replay.
fn sync_drum_channels(synth: &SynthPool, shared: &SharedState) {
    let mut mask = 0u64;
    for port in 0..synth.port_count() {
        for ch in 0..16usize {
            if synth.is_percussion_channel(port, ch) {
                mask |= 1u64 << (port as u64 * 16 + ch as u64);
            }
        }
    }
    shared.drum_channels.store(mask, Ordering::Relaxed);
}
