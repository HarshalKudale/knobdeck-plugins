# Hardware Monitor

Live readings from your computer's sensors on a key or a dial: CPU and GPU temperatures, CPU
package power, GPU load, VRAM and power, fan speeds, and any other sensor the machine reports.
Works on Windows, macOS and Linux. Everything is read-only.

## What it does

- **Sensor**: one reading on a key (`CPU` / `54 °C`). Pick an automatic sensor (CPU temperature,
  CPU package power, GPU temperature / load / memory / power / fan, system fan), which finds the
  right sensor on any computer, or any single sensor from the list. Optional alert: the key turns
  red at or above a value you choose.
- **Sensor Gauge**: the same reading as a ring gauge. The scale is automatic (20–100 °C, 0–100 %,
  the GPU's power limit, the VRAM size, the fan's top speed) or your own.
- **Two Sensors**: two readings on one key (CPU and GPU temperature by default).

Temperatures show in °C or °F. CPU use, GPU use and memory use as percentages are in Windows /
Mac / Linux Tools' System Stats; this plugin is about the sensors.

## Setup

- **Linux**: none. Sensors come from the kernel (`/sys/class/hwmon`): CPU (k10temp, coretemp,
  zenpower), NVMe and SATA drives, memory, mainboard chips with a loaded driver (`sensors-detect`
  from lm-sensors finds the module for yours), AMD GPUs (with load and VRAM). CPU package power
  comes from RAPL when your kernel lets users read it (most make it root-only); on AMD APUs the
  integrated GPU's PPT reading is used instead.
- **Windows**: Windows offers no sensor API to apps, so for CPU, mainboard and drive sensors
  install and run [LibreHardwareMonitor](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor)
  (or OpenHardwareMonitor); the plugin reads what it publishes. Without it, keys say so; ACPI
  thermal zones are used when Windows lets a normal app read them.
- **macOS**: none. Apple silicon temperatures come from the system's HID sensors; fans and power
  (and Intel Macs' temperatures) from the SMC.
- **NVIDIA GPUs** (Windows, Linux): read through NVML, which the NVIDIA driver installs.

## How it works

- Linux: hwmon sysfs, the amdgpu device files (`gpu_busy_percent`, `mem_info_vram_*`), RAPL
  (`/sys/class/powercap`).
- Windows: WMI `root\LibreHardwareMonitor` / `root\OpenHardwareMonitor`, `root\WMI`
  `MSAcpi_ThermalZoneTemperature`.
- macOS: `IOHIDEventSystemClient` (Apple silicon temperatures; resolved at run time) and the
  AppleSMC user client through IOKit.
- NVIDIA: `nvml.dll` / `libnvidia-ml.so.1`, loaded at run time.

Readings refresh every 2 seconds; every key shares one scan.
