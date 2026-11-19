//! Lists this computer's MIDI ports: `cargo run -p midi-io --example ports`.
fn main() {
    println!("outputs: {:?}", midi_io::outputs());
    println!("inputs:  {:?}", midi_io::inputs());
}
