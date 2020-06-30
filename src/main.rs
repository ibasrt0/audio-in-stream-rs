use cpal::traits::{DeviceTrait, EventLoopTrait, HostTrait};

/// map range [min,max] to [0,1]
fn normalize(x: f32, min: f32, max: f32) -> f32 {
    assert!(min != max);
    (x - min) / (max - min)
}

/// map range [0,1] to [min,max]
fn denormalize(x: f32, min: f32, max: f32) -> f32 {
    (max - min) * x + min
}

/// map i16 range to [-1,+1]
fn signed_normalize_i16(x: i16) -> f32 {
    denormalize(
        normalize(x as f32, i16::MIN as f32, i16::MAX as f32),
        -1.0,
        1.0,
    )
}

fn root_mean_square<'a, I>(values: I) -> f32
where
    I: IntoIterator<Item = &'a i16>,
{
    // let rms = (it.map(|s| signed_normalize_i16(*s).powi(2)).sum::<f32>()
    //     / (num_samples as f32))
    //     .sqrt();

    let mut n: usize = 0;
    let mut square_sum: f32 = 0.0;
    for x in values {
        n += 1;
        square_sum += signed_normalize_i16(*x).powi(2);
    }

    (square_sum / n as f32).sqrt()
}

/// Compute dBov unit of decibels relative to full scale.
/// RMS value of a full-scale square wave is designated 0 dBov.
/// All possible dBov measurements are negative numbers.
/// To prevent an undefined log10(0), this implementation has a minimal value of 20*log10(1/i16::MAX).
#[allow(non_snake_case)]
fn dBov<'a, I>(values: I) -> f32
where
    I: IntoIterator<Item = &'a i16>,
{
    let rms = root_mean_square(values);
    let min = 1.0 / i16::MAX as f32;
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

/// print all CPAL input devices in all CPAL hosts that support the sample format
fn print_cpal_input_devices_with_sample_format(sample_format: &cpal::Format) {
    // TODO: move out this block to its own function
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
                            if f.channels == sample_format.channels
                                && f.max_sample_rate >= sample_format.sample_rate
                                && f.data_type == sample_format.data_type
                            {
                                println!(
                                    "host: '{}', input_device: '{}'",
                                    host_id.name(),
                                    dev.name().unwrap_or_else(|_| String::from(
                                        "<failed to get device name>"
                                    ))
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}
fn main() {
    // assume CD Audio sample format
    let sample_format = cpal::Format {
        channels: 2,
        sample_rate: cpal::SampleRate(44100),
        data_type: cpal::SampleFormat::I16,
    };

    // command line arg to list all input devices in all hosts that support the sample format
    if let Some(_) = std::env::args().find(|arg| arg == "--list-input-devices") {
        print_cpal_input_devices_with_sample_format(&sample_format);
        return;
    }

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
        // assume i16 samples
        if let cpal::StreamData::Input {
            buffer: cpal::UnknownTypeInputBuffer::I16(input_buffer),
        } = stream_result.expect("input stream error")
        {
            if !first_line && is_tty {
                // up one line
                print!("\x1b[1A");
            }

            let num_channels = sample_format.channels as usize;
            assert!(input_buffer.len() % num_channels == 0);
            let num_samples = input_buffer.len() / num_channels;
            print!(
                "input buffer: {} i16 samples * {} channel(s), {:7.3} ms",
                num_samples,
                num_channels,
                1000.0 * num_samples as f32 / sample_format.sample_rate.0 as f32,
            );

            for channel_index in 0..num_channels {
                // each channel data is interleaved
                let it = input_buffer
                    .iter()
                    .skip(channel_index)
                    .step_by(num_channels);

                #[allow(non_snake_case)]
                let channel_dBov = dBov(it);
                #[allow(non_snake_case)]
                let minimal_dBov = dBov(&vec![0]);

                print!(
                    ", channel {}:▕{}▏{:+4.1} dBov",
                    channel_index,
                    vertical_scale_char(1.0 - channel_dBov / minimal_dBov),
                    channel_dBov
                );
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
    });
    // speaker-test  -c2 -l1
}