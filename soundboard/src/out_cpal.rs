//! Output devices on Windows (WASAPI) and macOS (CoreAudio), through cpal.

use rodio::cpal::traits::HostTrait;
use rodio::mixer::Mixer;
use rodio::{DeviceSinkBuilder, DeviceTrait, MixerDeviceSink};

/// An open output: sounds are added to its mixer. Dropping it closes the stream.
pub struct Output {
    sink: MixerDeviceSink,
}

impl Output {
    pub fn mixer(&self) -> &Mixer {
        self.sink.mixer()
    }
}

fn name(d: &rodio::Device) -> String {
    d.description().map(|d| d.name().to_string()).unwrap_or_else(|_| d.to_string())
}

/// `(id, name)` of every output device. The id is cpal's stable device id ("wasapi:…",
/// "coreaudio:…").
pub fn devices() -> Result<Vec<(String, String)>, String> {
    let host = rodio::cpal::default_host();
    let list = host.output_devices().map_err(|e| format!("Can't list the output devices: {e}"))?;
    Ok(list.filter_map(|d| Some((d.id().ok()?.to_string(), name(&d)))).collect())
}

pub fn default_name() -> Option<String> {
    rodio::cpal::default_host().default_output_device().map(|d| name(&d))
}

fn device(id: &str) -> Result<rodio::Device, String> {
    let host = rodio::cpal::default_host();
    if id == crate::engine::DEFAULT {
        return host.default_output_device().ok_or_else(|| "This computer has no output device".to_string());
    }
    let parsed: rodio::cpal::DeviceId = id.parse().map_err(|_| format!("Unknown output device \"{id}\": choose it again"))?;
    host.device_by_id(&parsed).ok_or_else(|| "The chosen output device isn't connected".to_string())
}

pub fn open(id: &str) -> Result<Output, String> {
    let dev = device(id)?;
    let label = name(&dev);
    let mut sink = DeviceSinkBuilder::from_device(dev)
        .and_then(|b| b.open_sink_or_fallback())
        .map_err(|e| format!("Can't open {label}: {e}"))?;
    sink.log_on_drop(false);
    Ok(Output { sink })
}
