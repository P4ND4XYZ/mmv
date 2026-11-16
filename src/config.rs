pub(crate) mod lua;

use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Position {
    Center,
    Left,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorScheme {
    Dark,
    Light,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Group {
    Normal,
    Border,
    Search,
    SearchCurrent,
    StatusLine,
    CommandLine,
    ModeNormal,
    ModeSearch,
    ModeCommand,
    Heading1,
    Heading2,
    Heading3,
    Heading4,
    Heading5,
    Heading6,
    Link,
    CodeBlock,
    Quote,
    Panel,
    PanelSelected,
    Error,
    Warning,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color(pub u8, pub u8, pub u8);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Highlight {
    pub fg: Option<Color>,
    pub bg: Option<Color>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeConfig {
    pub visible: bool,
    pub root_markers: Vec<String>,
}

impl Default for TreeConfig {
    fn default() -> Self {
        Self {
            visible: false,
            root_markers: vec![".marksman.toml".into(), ".git".into()],
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub position: Position,
    pub max_width: f64,
    pub font_size: f32,
    pub colorscheme: ColorScheme,
    pub highlights: BTreeMap<Group, Highlight>,
    pub tree: TreeConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            position: Position::Center,
            max_width: 960.0,
            font_size: 16.0,
            colorscheme: ColorScheme::Dark,
            highlights: BTreeMap::new(),
            tree: TreeConfig::default(),
        }
    }
}

#[cfg(test)]
pub fn execute(source: &str, name: &str) -> Result<Config, String> {
    let (runtime, error) = crate::lua::LuaRuntime::initialize("", Some((source, name)))
        .map_err(|error| error.to_string())?;
    match error {
        Some(error) => Err(error),
        None => Ok(runtime.config()),
    }
}

#[cfg(test)]
mod tests;
