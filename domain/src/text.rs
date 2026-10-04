//! Counts the characters a length limit governs.

/// Counts the Unicode scalar values in a string.
///
/// A limit expressed in bytes would reject a title an accented letter makes two
/// bytes longer while accepting a longer one made of ASCII, and `len` on a Rust
/// `str` is bytes. Counting `chars` counts scalar values, which puts the limit
/// where a reader would put it and in the same place as the Go, C# and Java
/// siblings, which count runes or code points.
#[must_use]
pub fn count_characters(value: &str) -> usize {
    value.chars().count()
}

#[cfg(test)]
mod tests {
    use super::count_characters;

    /// One scalar value, two bytes. The distinction is the whole point.
    const ACCENTED: &str = "é";
    const EMOJI: &str = "\u{1F600}";
    const LATIN: &str = "ab";

    const BYTES_IN_THE_ACCENTED_LETTER: usize = 2;
    const BYTES_IN_THE_EMOJI: usize = 4;
    const CHARACTERS_IN_THE_LATIN_PAIR: usize = 2;

    #[test]
    fn counts_a_multibyte_character_once() {
        assert_eq!(ACCENTED.len(), BYTES_IN_THE_ACCENTED_LETTER);
        assert_eq!(count_characters(ACCENTED), 1, "one character");
    }

    #[test]
    fn counts_a_character_outside_the_basic_plane_once() {
        assert_eq!(EMOJI.len(), BYTES_IN_THE_EMOJI);
        assert_eq!(count_characters(EMOJI), 1, "one character");
    }

    #[test]
    fn counts_each_latin_character_once() {
        assert_eq!(count_characters(LATIN), CHARACTERS_IN_THE_LATIN_PAIR);
    }

    #[test]
    fn counts_nothing_in_an_empty_string() {
        assert_eq!(count_characters(""), 0);
    }
}
