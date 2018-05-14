use rand::{OsRng, Rng};
use std;
use std::str;

pub fn random_bytes(bytes: &mut [u8]) {
    let mut rng = OsRng::new().unwrap();
    rng.fill_bytes(bytes);
}

// bytestring to hex representation of each byte as two characters.
// so the resulting string's size is 2x the size of the input bytestring
pub fn bytes_to_hexstr(b: &[u8]) -> String {
    let hexstr: Vec<String> = b.iter().map(|c| format!("{:02x}", c)).collect();

    hexstr.join("")
}

#[allow(dead_code)] // TODO: Drop this once function is being used
pub fn hex_to_char(s: &str) -> Result<char, std::num::ParseIntError> {
    u8::from_str_radix(s, 16).map(|n| n as char)
}

#[allow(dead_code)] // TODO: Drop this once function is being used
pub fn hexstr_to_string(hexstr: &str) -> String {
    let chars: Vec<&str> = hexstr
        .as_bytes()
        .chunks(2)
        .map(|ch| str::from_utf8(ch).unwrap())
        .collect();

    let s: Vec<char> = chars
        .iter()
        .map(|x| hex_to_char(x).unwrap())
        .collect();

    s.iter().collect::<String>()
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_bytes_to_hexstr() {
        let s1 = b"I am a String";
        assert_eq!(
            bytes_to_hexstr(b"I am a String"),
            "4920616d206120537472696e67"
        );
    }

    #[test]
    fn test_hexstr_to_string() {
        let s1 = "7b2270616b655f7631223a22353337363331646366643064336164386130346234663531643935336131343563386538626663373830646461393834373934656634666136656536306339663665227d";
        assert_eq!(hexstr_to_string(s1), "{\"pake_v1\":\"537631dcfd0d3ad8a04b4f51d953a145c8e8bfc780dda984794ef4fa6ee60c9f6e\"}");
    }
}
