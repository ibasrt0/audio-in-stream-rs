// Copyright 2020  Israel Basurto
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as
// published by the Free Software Foundation, either version 3 of the
// License, or (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

use cpal::traits::{DeviceTrait, EventLoopTrait, HostTrait};
use std::sync::{Arc, RwLock};
use std::thread;

fn clamp(x: f32, min: f32, max: f32) -> f32 {
    x.max(min).min(max)
}

fn root_mean_square<'a>(values: impl IntoIterator<Item = &'a f32>) -> f32 {
    let mut n: usize = 0;
    let mut square_sum: f32 = 0.0;
    for x in values {
        n += 1;
        square_sum += x.powi(2);
    }

    (square_sum / n as f32).sqrt()
}

/// Given a loudness level in nominal interval of [0,+1],
/// compute dBov unit of decibels relative to overload.
/// A loundness level of 1 is designated as 0 dBov and
/// a loundness level of 0 is designated as -inf.
/// Loudness level is usually computed as the root mean square of
/// a audio signal in the nominal interval of [-1,+1]
fn decibels_overload<'a>(loudness_level: f32) -> f32 {
    20.0 * loudness_level.log10()
}

fn quantization_noise_ratio(quantization_bits: usize) -> f32 {
    20.0 * 2.0_f32.log10() * quantization_bits as f32
}

fn horizontal_scale(value: f32, num_chars: usize) -> String {
    let mut hscale = String::with_capacity(num_chars);
    let normalized_value = clamp(value, 0.0, 1.0);
    let ivalue = (normalized_value * num_chars as f32) as usize;
    for i in 0..num_chars {
        if i < ivalue {
            hscale.push('=');
        } else {
            hscale.push(' ');
        }
    }
    hscale
}

/// print all supported sample formats in all CPAL input devices in all CPAL hosts
fn print_cpal_input_devices() {
    let default_host = cpal::default_host();
    if let Some(dev) = default_host.default_input_device() {
        println!(
            "default: host: '{}', input_device: '{}'",
            default_host.id().name(),
            dev.name()
                .unwrap_or_else(|_| String::from("<failed to get device name>"))
        );
    } else {
        println!("default: host: '{}'", default_host.id().name());
    }

    for host_id in cpal::available_hosts() {
        if let Ok(host) = cpal::host_from_id(host_id) {
            if let Ok(input_devices) = host.input_devices() {
                for dev in input_devices {
                    if let Ok(supported_input_formats) = dev.supported_input_formats() {
                        for f in supported_input_formats {
                            println!(
                                    "host: '{}', input_device: '{}' channels: {}, sample rate min: {} max: {}, {:?}",
                                    host_id.name(),
                                    dev.name().unwrap_or_else(|_| String::from(
                                        "<failed to get device name>"
                                    )),
                                    f.channels,
                                    f.min_sample_rate.0,
                                    f.max_sample_rate.0,
                                    f.data_type
                                );
                        }
                    }
                }
            }
        }
    }
}

struct ChannelData {
    rms: f32,
    decibels_overload: f32,
    samples: Vec<f32>,
}

fn process_input_buffer<T: cpal::Sample>(
    input_buffer: cpal::InputBuffer<T>,
    sample_format: &cpal::Format,
) -> Vec<ChannelData> {
    let num_channels = sample_format.channels as usize;
    assert!(num_channels > 0);
    assert!(input_buffer.len() % num_channels == 0);
    let mut channel_data = Vec::with_capacity(num_channels);

    for channel_index in 0..num_channels {
        let samples: Vec<_> = input_buffer
            .iter()
            // each channel data is interleaved
            .skip(channel_index)
            .step_by(num_channels)
            .map(|s| s.to_f32())
            .collect();

        let rms = root_mean_square(&samples);
        channel_data.push(ChannelData {
            rms: rms,
            decibels_overload: decibels_overload(rms),
            samples: samples,
        });
    }

    channel_data
}

fn main() {
    // assume CD Audio sample format
    let sample_config = cpal::Format {
        channels: 2,
        sample_rate: cpal::SampleRate(44100),
        // data_type: cpal::SampleFormat::I16,
        data_type: cpal::SampleFormat::F32,
    };

    // command line arg to list all supported the sample format in all input devices in all hosts
    if let Some(_) = std::env::args().find(|arg| arg == "--list-input-devices") {
        print_cpal_input_devices();
        return;
    }

    // audio input thread
    let input_buffer_info_rwlock = Arc::new(RwLock::new(String::new()));
    let input_buffer_info_wlock = input_buffer_info_rwlock.clone();
    thread::spawn(move || {
        let host = cpal::default_host();
        // TODO: allow user select different device
        let dev = host
            .default_input_device()
            .expect("failed to get default input device");
        let event_loop = host.event_loop();

        let stream_id = event_loop
            .build_input_stream(&dev, &sample_config)
            .expect("failed to build input stream, maybe invalid input device");

        event_loop
            .play_stream(stream_id)
            .expect("failed to play stream");

        let is_tty = atty::is(atty::Stream::Stdout);
        let mut first_line = true;

        event_loop.run(move |_stream_id, stream_result| {
            if let cpal::StreamData::Input { buffer } = stream_result.expect("input stream error") {
                if !first_line && is_tty {
                    // up one line
                    print!("\x1b[1A");
                }

                let (num_samples, channel_data, sample_format) = match buffer {
                    cpal::UnknownTypeInputBuffer::U16(input_buffer) => (
                        input_buffer.len(),
                        process_input_buffer(input_buffer, &sample_config),
                        cpal::SampleFormat::U16,
                    ),
                    cpal::UnknownTypeInputBuffer::I16(input_buffer) => (
                        input_buffer.len(),
                        process_input_buffer(input_buffer, &sample_config),
                        cpal::SampleFormat::I16,
                    ),
                    cpal::UnknownTypeInputBuffer::F32(input_buffer) => (
                        input_buffer.len(),
                        process_input_buffer(input_buffer, &sample_config),
                        cpal::SampleFormat::F32,
                    ),
                };

                let mut input_buffer_info = format!(
                    "input buffer: {:>6} {:#?} samples * {} channel(s), {:>7.3} ms",
                    num_samples / channel_data.len(),
                    sample_format,
                    channel_data.len(),
                    1000.0 * num_samples as f32 / sample_config.sample_rate.0 as f32
                );

                for (channel_index, channel) in channel_data.iter().enumerate() {
                    input_buffer_info += &format!(
                        ", channel {}: [{}] {:>+5.1} dBov",
                        channel_index,
                        // horizontal scale from 0 dBov
                        // to the quantization noise level for 16 bits, i.e. ~96 dB
                        // (a reasonable bottom level, regardless the bit deep of
                        // the samples)
                        // Also, using 16 chars in the horizontal scale
                        // make each char position an indication of a 1 bit
                        // or ~6 dB, equivalent of factor of change in value relative
                        // to the previous/next char position of 0.5
                        horizontal_scale(
                            1.0 + channel.decibels_overload / quantization_noise_ratio(16),
                            16
                        ),
                        channel.decibels_overload,
                    );
                }

                print!("{}", input_buffer_info);
                {
                    *input_buffer_info_wlock.write().unwrap() = input_buffer_info;
                }

                if is_tty {
                    // clear the rest of the line
                    print!("\x1b[0K");
                }

                println!();
                first_line = false;
            } else {
                unimplemented!("invalid audio stream input/output format");
            }
        })
    });

    // main thread, http server
    use tiny_http::{Response, Server};
    let server = Server::http("0.0.0.0:8000").unwrap();

    for request in server.incoming_requests() {
        let response = if request.url() == "/info" {
            Response::from_string(format!(
                include_str!("pre-reload.html"),
                *input_buffer_info_rwlock.read().unwrap()
            ))
            .with_header(
                tiny_http::Header::from_bytes(
                    &b"Content-Type"[..],
                    &b"text/html; charset=UTF-8"[..],
                )
                .unwrap(),
            )
        } else {
            Response::from_string(format!(
                "received request!\nmethod: {:?}\nurl: {:?}\nheaders: {:?}",
                request.method(),
                request.url(),
                request.headers()
            ))
        };

        request.respond(response).unwrap();
    }

    // tested with 'speaker-test -c2 -l1' in a loopback
    // (audio output connected to the audio input)
}
