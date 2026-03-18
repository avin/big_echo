pub fn default_text_editor_id() -> Option<&'static str> {
    if cfg!(target_os = "macos") {
        Some("textedit")
    } else if cfg!(target_os = "windows") {
        Some("notepad")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_default_for_supported_platforms() {
        if cfg!(target_os = "macos") {
            assert_eq!(default_text_editor_id(), Some("textedit"));
        } else if cfg!(target_os = "windows") {
            assert_eq!(default_text_editor_id(), Some("notepad"));
        } else {
            assert_eq!(default_text_editor_id(), None);
        }
    }
}
