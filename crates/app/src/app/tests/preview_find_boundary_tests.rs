use crate::app::preview::{find_matches, floor_char_boundary};

/// Stale find offsets may land inside a multi-byte character; jumps must clamp down to a character boundary first.
#[test]
fn floor_char_boundary_never_lands_inside_a_multibyte_char() {
    // Fixture: "ab" + two 3-byte CJK glyphs + "cd" — a=0 b=1 | glyph 1 = 2..5 | glyph 2 = 5..8 | c=8 d=9, len=10.
    // Character boundaries: 0,1,2,5,8,9,10; every other offset falls inside glyph 1 or glyph 2.
    let text = "ab模型cd";
    assert_eq!(floor_char_boundary(text, 0), 0);
    assert_eq!(floor_char_boundary(text, 2), 2, "字符起点不动");
    for byte in [3, 4, 6, 7] {
        let bounded = floor_char_boundary(text, byte);
        assert!(
            text.is_char_boundary(bounded),
            "byte {byte} 钳到 {bounded}，必须落在字符边界"
        );
        assert!(bounded < byte, "只向下钳，不会跳到后面");
    }
    assert_eq!(floor_char_boundary(text, 8), 8, "ASCII 段内不动");
    assert_eq!(floor_char_boundary(text, 10), 10, "恰好等于长度是合法边界");
    assert_eq!(floor_char_boundary(text, 999), text.len(), "超界钳到长度内");
}

/// Offsets find returns are always character boundaries of the matched text.
#[test]
fn find_matches_offsets_are_always_char_boundaries_of_the_text_they_matched() {
    let text = "前缀模型后缀 模型 再一个模型";
    for (start, end) in find_matches(text, "模型") {
        assert!(text.is_char_boundary(start) && text.is_char_boundary(end));
        assert_eq!(&text[start..end], "模型");
    }
}
