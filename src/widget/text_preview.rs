use ansi_to_tui::IntoText;
use once_cell::sync::Lazy;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    text::Line,
    widgets::{Block, StatefulWidget},
};
use syntect::{
    easy::HighlightLines,
    highlighting::ThemeSet,
    parsing::SyntaxSet,
    util::{as_24_bit_terminal_escaped, LinesWithEndings},
};

use crate::{
    color::ColorTheme,
    config::Config,
    object::{FileDetail, RawObject},
    ui::common::format_version,
    util::extension_from_file_name,
    widget::{ScrollLines, ScrollLinesOptions, ScrollLinesState},
};

static SYNTAX_SET: Lazy<SyntaxSet> = Lazy::new(|| {
    if let Ok(path) = Config::preview_syntax_dir_path() {
        if path.exists() {
            // SyntaxSetBuilder::build is terribly slow in debug build...
            // To avoid unnecessary processing, we won't use the builder if the syntax directory doesn't exist...
            let mut builder = SyntaxSet::load_defaults_newlines().into_builder();
            builder.add_from_folder(path, true).unwrap();
            builder.build()
        } else {
            SyntaxSet::load_defaults_newlines()
        }
    } else {
        SyntaxSet::load_defaults_newlines()
    }
});
static DEFAULT_THEME_SET: Lazy<ThemeSet> = Lazy::new(ThemeSet::load_defaults);
static USER_THEME_SET: Lazy<ThemeSet> = Lazy::new(|| {
    Config::preview_theme_dir_path()
        .and_then(|path| ThemeSet::load_from_folder(path).map_err(Into::into))
        .unwrap_or_default()
});

#[derive(Debug)]
pub struct TextPreviewState {
    pub scroll_lines_state: ScrollLinesState,
}

impl TextPreviewState {
    pub fn new(
        file_detail: &FileDetail,
        object: &RawObject,
        highlight: bool,
        highlight_theme_name: &str,
    ) -> (Self, Option<String>) {
        let mut warn_msg = None;

        let s = to_preview_string(&object.bytes);

        let lines: Vec<Line<'static>> =
            match build_highlighted_lines(&s, &file_detail.name, highlight, highlight_theme_name) {
                Ok(lines) => lines,
                Err(msg) => {
                    // If there is an error, display the original text
                    if let Some(msg) = msg {
                        warn_msg = Some(msg);
                    }
                    s.lines().map(drop_control_chars).map(Line::raw).collect()
                }
            };

        let scroll_lines_state = ScrollLinesState::new(lines, ScrollLinesOptions::default());

        let state = Self { scroll_lines_state };
        (state, warn_msg)
    }
}

fn to_preview_string(bytes: &[u8]) -> String {
    let s: String = String::from_utf8_lossy(bytes).into();
    // tab is not rendered correctly, so replace it
    let s = s.replace('\t', "    ");
    if s.ends_with('\n') {
        s.trim_end().into()
    } else {
        s
    }
}

fn drop_control_chars(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).collect()
}

fn build_highlighted_lines(
    s: &str,
    file_name: &str,
    highlight: bool,
    highlight_theme_name: &str,
) -> Result<Vec<Line<'static>>, Option<String>> {
    if !highlight {
        return Err(None);
    }

    let extension = extension_from_file_name(file_name);
    let syntax = SYNTAX_SET
        .find_syntax_by_extension(&extension)
        .ok_or_else(|| {
            let msg = format!("No syntax definition found for `.{}`", extension);
            Some(msg)
        })?;
    let theme = &DEFAULT_THEME_SET
        .themes
        .get(highlight_theme_name)
        .or_else(|| USER_THEME_SET.themes.get(highlight_theme_name))
        .ok_or_else(|| {
            let msg = format!("Theme `{}` not found", highlight_theme_name);
            Some(msg)
        })?;
    let mut h = HighlightLines::new(syntax, theme);
    let s = LinesWithEndings::from(s)
        .map(|line| {
            let ranges: Vec<(syntect::highlighting::Style, &str)> =
                h.highlight_line(line, &SYNTAX_SET).unwrap();
            as_24_bit_terminal_escaped(&ranges[..], false)
        })
        .collect::<Vec<String>>()
        .join("");
    Ok(s.into_text().unwrap().into_iter().collect())
}

#[derive(Debug)]
pub struct TextPreview<'a> {
    file_name: &'a str,
    file_version_id: Option<&'a str>,

    theme: &'a ColorTheme,
}

impl<'a> TextPreview<'a> {
    pub fn new(
        file_name: &'a str,
        file_version_id: Option<&'a str>,
        theme: &'a ColorTheme,
    ) -> Self {
        Self {
            file_name,
            file_version_id,
            theme,
        }
    }
}

impl StatefulWidget for TextPreview<'_> {
    type State = TextPreviewState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        let title = if let Some(version_id) = self.file_version_id {
            format!(
                "Preview [{} (Version ID: {})]",
                self.file_name,
                format_version(version_id)
            )
        } else {
            format!("Preview [{}]", self.file_name)
        };
        ScrollLines::default()
            .block(Block::bordered().title(title))
            .theme(self.theme)
            .render(area, buf, &mut state.scroll_lines_state);
    }
}
