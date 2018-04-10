
// bytestring to hex representation of each byte as two characters.
// so the resulting string's size is 2x the size of the input bytestring
pub fn bytes_to_hexstr(b: &[u8]) -> String {
    let hexstr: Vec<String> = b.iter()
        .map(|c| format!("{:02x}", c))
        .collect();

    hexstr.join("")
}
