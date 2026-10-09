use crate::app::preview::{find_matches, floor_char_boundary};

/// 过期查找偏移可能落在多字节字符中间，跳转前必须向下钳回字符边界。
#[test]
fn floor_char_boundary_never_lands_inside_a_multibyte_char() {
    // "ab模型cd"：a=0 b=1 | 模=2..5 | 型=5..8 | c=8 d=9，len=10。
    // 字符边界：0,1,2,5,8,9,10；其余都落在「模」或「型」内部。
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

/// 查找返回的偏移始终是所匹配文本的字符边界。
#[test]
fn find_matches_offsets_are_always_char_boundaries_of_the_text_they_matched() {
    let text = "前缀模型后缀 模型 再一个模型";
    for (start, end) in find_matches(text, "模型") {
        assert!(text.is_char_boundary(start) && text.is_char_boundary(end));
        assert_eq!(&text[start..end], "模型");
    }
}
