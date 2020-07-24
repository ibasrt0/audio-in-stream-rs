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

/// Compute dBov unit of decibels relative to full scale.
/// RMS value of a full-scale square wave is designated 0 dBov.
/// All possible dBov measurements are negative numbers.
/// To prevent an undefined log10(0), this implementation has
/// a minimal value of 20*log10(f32::EPSILON).
#[allow(non_snake_case)]
fn dBov<'a>(values: impl IntoIterator<Item = &'a f32>) -> f32 {
    let rms = root_mean_square(values);
    let min = f32::EPSILON;
    20.0 * rms.max(min).log10()
}

fn vertical_scale_char(normalized_value: f32) -> char {
    assert!(normalized_value >= 0.0 && normalized_value <= 1.0);
    let vblock_chars = " ▁▂▃▄▅▆▇█";
    let last = vblock_chars.chars().count() - 1;
    vblock_chars
        .chars()
        .nth((last as f32 * normalized_value).round() as usize)
        .unwrap()
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

fn process_input_buffer<T: cpal::Sample>(
    input_buffer: cpal::InputBuffer<T>,
    sample_format: &cpal::Format,
) -> String
{
    let num_channels = sample_format.channels as usize;
    assert!(input_buffer.len() % num_channels == 0);
    let num_samples = input_buffer.len() / num_channels;

    let mut input_buffer_info = format!(
        "input buffer: {} {:#?} samples * {} channel(s), {:7.3} ms",
        num_samples,
        T::get_format(),
        num_channels,
        1000.0 * num_samples as f32 / sample_format.sample_rate.0 as f32,
    );

    for channel_index in 0..num_channels {
        let input_buffer_f32: Vec<_> = input_buffer
            .iter()
            // each channel data is interleaved
            .skip(channel_index)
            .step_by(num_channels)
            .map(|s| s.to_f32())
            .collect();

        #[allow(non_snake_case)]
        let channel_dBov = dBov(&input_buffer_f32);
        #[allow(non_snake_case)]
        let minimal_dBov = dBov(&vec![0.0]);

        input_buffer_info += &format!(
            ", channel {}: {} {:+4.1} dBov",
            channel_index,
            vertical_scale_char(clamp(1.0 - channel_dBov / minimal_dBov, 0.0, 1.0)),
            channel_dBov
        );
    }

    input_buffer_info
}

fn main() {
    // assume CD Audio sample format
    let sample_format = cpal::Format {
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
            .build_input_stream(&dev, &sample_format)
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

                let input_buffer_info = match buffer {
                    cpal::UnknownTypeInputBuffer::U16(input_buffer) => {
                        process_input_buffer(input_buffer, &sample_format)
                    }
                    cpal::UnknownTypeInputBuffer::I16(input_buffer) => {
                        process_input_buffer(input_buffer, &sample_format)
                    }
                    cpal::UnknownTypeInputBuffer::F32(input_buffer) => {
                        process_input_buffer(input_buffer, &sample_format)
                    }
                };

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
