//! Lists input devices and opens each named one for about a second (smoke check for capture).
//!   cargo run --release -p engine-native --example mic_probe [device substring] [rate] [--panic]
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

fn main() {
    std::panic::set_hook(Box::new(|p| eprintln!("PANIC: {p}")));
    let host = cpal::default_host();
    let default = host.default_input_device().and_then(|d| d.name().ok());
    println!("default input: {default:?}");
    for d in host.input_devices().unwrap() {
        let name = d.name().unwrap_or_default();
        let def = d
            .default_input_config()
            .map(|c| format!("{c:?}"))
            .unwrap_or_else(|e| e.to_string());
        println!("- {name}: default {def}");
    }
    let mut args: Vec<String> = std::env::args().collect();
    // `--panic` panics inside the first audio callback: the process must survive it.
    let panic_once = args.iter().any(|a| a == "--panic");
    args.retain(|a| a != "--panic");
    let Some(want) = args.get(1) else { return };
    let rate: Option<u32> = args.get(2).and_then(|r| r.parse().ok());
    let dev = host
        .input_devices()
        .unwrap()
        .find(|d| d.name().map(|n| n.contains(want.as_str())).unwrap_or(false))
        .expect("device");
    let mut cfg = dev.default_input_config().unwrap();
    if let Some(r) = rate {
        cfg = dev
            .supported_input_configs()
            .unwrap()
            .filter(|c| c.sample_format() == cfg.sample_format())
            .find(|c| c.min_sample_rate().0 <= r && r <= c.max_sample_rate().0)
            .expect("rate")
            .with_sample_rate(cpal::SampleRate(r));
    }
    println!("opening {:?} with {cfg:?}", dev.name());
    let frames = Arc::new(AtomicUsize::new(0));
    let calls = Arc::new(AtomicUsize::new(0));
    let (f2, c2) = (frames.clone(), calls.clone());
    let ch = cfg.channels() as usize;
    let stream = dev
        .build_input_stream_raw(
            &cfg.config(),
            cfg.sample_format(),
            move |data: &cpal::Data, _| {
                let n = c2.fetch_add(1, Ordering::Relaxed);
                if panic_once && n == 0 {
                    panic!("deliberate panic in the input callback");
                }
                f2.fetch_add(data.len() / ch, Ordering::Relaxed);
            },
            |e| eprintln!("stream error: {e}"),
            None,
        )
        .unwrap();
    stream.play().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(1200));
    drop(stream);
    println!(
        "received {} frames in {} callbacks",
        frames.load(Ordering::Relaxed),
        calls.load(Ordering::Relaxed)
    );
}
