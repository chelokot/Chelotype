#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminalRgb {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

impl TerminalRgb {
    fn from_hex(hex: &str) -> Self {
        let red = u8::from_str_radix(&hex[1..3], 16).expect("valid terminal palette red");
        let green = u8::from_str_radix(&hex[3..5], 16).expect("valid terminal palette green");
        let blue = u8::from_str_radix(&hex[5..7], 16).expect("valid terminal palette blue");
        Self { red, green, blue }
    }

    pub fn red_unit(self) -> f64 {
        f64::from(self.red) / 255.0
    }

    pub fn green_unit(self) -> f64 {
        f64::from(self.green) / 255.0
    }

    pub fn blue_unit(self) -> f64 {
        f64::from(self.blue) / 255.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminalPalette {
    pub foreground: &'static str,
    pub background: &'static str,
    pub cursor: &'static str,
    pub indexed: [&'static str; 16],
}

impl TerminalPalette {
    pub fn background_rgb(self) -> TerminalRgb {
        TerminalRgb::from_hex(self.background)
    }
}

pub const GNOME_DARK: TerminalPalette = TerminalPalette {
    foreground: "#ffffff",
    background: "#1c1c1f",
    cursor: "#ffffff",
    indexed: [
        "#241f31", "#c01c28", "#2ec27e", "#f5c211", "#1e78e4", "#9841bb", "#0ab9dc", "#c0bfbc",
        "#5e5c64", "#ed333b", "#57e389", "#f8e45c", "#51a1ff", "#c061cb", "#4fd2fd", "#f6f5f4",
    ],
};

pub fn default_terminal_palette() -> &'static TerminalPalette {
    &GNOME_DARK
}
