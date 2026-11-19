//! Sends hex bytes to a MIDI port: `cargo run -p midi-io --example send -- "Midi Through" "90 3C 64"`.
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [port, hex] = args.as_slice() else {
        eprintln!("usage: send <port> <hex bytes>");
        std::process::exit(2);
    };
    let bytes = midi_io::msg::parse_hex(hex).expect("hex bytes");
    let out = midi_io::Output::connect("KnobDeck send", port).unwrap_or_else(|e| panic!("{e}"));
    for m in midi_io::msg::split(&bytes).unwrap_or_else(|e| panic!("{e}")) {
        out.send(&m).unwrap_or_else(|e| panic!("{e}"));
    }
}
