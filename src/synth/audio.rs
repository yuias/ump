//! Audio output via cpal.

use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::Stream;

use super::engine::SynthEngine;
use crate::sequencer::Sequencer;
use crate::state::SharedState;

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
    let rate = default_config.sample_rate().0;
    log_info!("Audio device: {:?}, sample_rate={}", device.name().unwrap_or_default(), rate);
    Ok(rate)
}

impl AudioOutput {
    /// Build and start the audio output stream.
    /// The stream runs the sequencer and synthesizer in the cpal callback.
    pub fn start(
        sequencer: Arc<Mutex<Sequencer>>,
        synth: Arc<Mutex<Option<SynthEngine>>>,
        shared: Arc<SharedState>,
        sample_rate: u32,
    ) -> Result<Self> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .context("No audio output device found")?;

        let config = cpal::StreamConfig {
            channels: 2,
            sample_rate: cpal::SampleRate(sample_rate),
            buffer_size: cpal::BufferSize::Default,
        };

        let channels = config.channels as usize;

        let err_fn = |err: cpal::StreamError| {
            log_error!("Audio stream error: {}", err);
        };

        let stream = device.build_output_stream(
            &config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                let sample_count = data.len() / channels;
                let mut left = vec![0.0f32; sample_count];
                let mut right = vec![0.0f32; sample_count];

                if let (Ok(mut seq), Ok(mut syn_opt)) =
                    (sequencer.try_lock(), synth.try_lock())
                {
                    if let Some(ref mut syn) = *syn_opt {
                        seq.fill_buffer(syn, &mut left, &mut right, &shared);
                    }
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
