//! Color output helpers that respect terminal capabilities

use crossterm::style::{Color, Stylize};
use std::fmt::Display;

/// Check if colors should be enabled
pub fn should_use_colors() -> bool {
    // Check if stdout is a terminal
    atty::is(atty::Stream::Stdout)
}

/// Trait for conditional coloring
pub trait ConditionalColor: Display + Sized {
    fn color_if(self, color: Color) -> StyledString<Self> {
        StyledString {
            content: self,
            color: if should_use_colors() { Some(color) } else { None },
            bold: false,
        }
    }
    
    fn bold_if(self) -> StyledString<Self> {
        StyledString {
            content: self,
            color: None,
            bold: should_use_colors(),
        }
    }
    
    fn color_bold_if(self, color: Color) -> StyledString<Self> {
        StyledString {
            content: self,
            color: if should_use_colors() { Some(color) } else { None },
            bold: should_use_colors(),
        }
    }
}

/// Implement for common types
impl ConditionalColor for &str {}
impl ConditionalColor for String {}
impl ConditionalColor for &String {}

/// Wrapper that conditionally applies styling
pub struct StyledString<T: Display> {
    content: T,
    color: Option<Color>,
    bold: bool,
}

impl<T: Display> Display for StyledString<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(color) = self.color {
            if self.bold {
                write!(f, "{}", self.content.to_string().with(color).bold())
            } else {
                write!(f, "{}", self.content.to_string().with(color))
            }
        } else if self.bold {
            write!(f, "{}", self.content.to_string().bold())
        } else {
            write!(f, "{}", self.content)
        }
    }
}