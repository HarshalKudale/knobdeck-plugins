## 1.8.0

- New audio actions, matching Win Tools and Linux Tools (same ids and settings): Device Volume
  slider, Mute Device, Set Audio Device, Toggle Audio Device and Push to Talk. Output volume uses
  the system's virtual main volume (AudioToolbox), else the device's own volume controls; devices
  are remembered by their CoreAudio UID.
