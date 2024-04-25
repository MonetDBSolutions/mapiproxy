pub struct Colors {
    pub normal: EscapeSequence,
    pub red: EscapeSequence,
    pub green: EscapeSequence,
    pub cyan: EscapeSequence,
    pub blue: EscapeSequence,
    pub bold: EscapeSequence,
}

#[derive(Debug, Clone, Copy)]
pub struct EscapeSequence {
    pub enable: &'static str,
    pub disable: &'static str,
}

impl EscapeSequence {
    pub const fn new(enable: &'static str, disable: &'static str) -> Self {
        EscapeSequence { enable, disable }
    }
}

const EMPTY: EscapeSequence = EscapeSequence::new("", "");

pub static NO_COLORS: &Colors = &Colors {
    normal: EMPTY,
    bold: EMPTY,
    red: EMPTY,
    green: EMPTY,
    cyan: EMPTY,
    blue: EMPTY,
};

#[allow(dead_code)]
pub static DEBUG_COLORS: &Colors = &Colors {
    normal: EscapeSequence::new("«-»", ""),
    red: EscapeSequence::new("«red»", ""),
    green: EscapeSequence::new("«green»", ""),
    cyan: EscapeSequence::new("«cyan»", ""),
    blue: EscapeSequence::new("«blue»", ""),
    bold: EscapeSequence::new("«bold»", "«/bold»"),
};

// Black=30 Red=31 Green=32 Yellow=33 Blue=34 Magenta=35 Cyan=36 White=37

pub static VT100_COLORS: &Colors = &Colors {
    normal: EscapeSequence::new("\u{1b}[39m", ""),
    red: EscapeSequence::new("\u{1b}[31m", ""),
    green: EscapeSequence::new("\u{1b}[32m", ""),
    cyan: EscapeSequence::new("\u{1b}[36m", ""),
    blue: EscapeSequence::new("\u{1b}[34m", ""),
    bold: EscapeSequence::new("\u{1b}[1m", "\u{1b}[0m"),
};
