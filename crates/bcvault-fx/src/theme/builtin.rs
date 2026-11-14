pub const BUILTIN_THEME_FILES: [(&str, &str); 10] = [
    ("carbon", include_str!("../../themes/carbon.toml")),
    ("paper", include_str!("../../themes/paper.toml")),
    ("terminal", include_str!("../../themes/terminal.toml")),
    ("vapor", include_str!("../../themes/vapor.toml")),
    ("midnight", include_str!("../../themes/midnight.toml")),
    ("ember", include_str!("../../themes/ember.toml")),
    (
        "catppuccin-mocha",
        include_str!("../../themes/catppuccin-mocha.toml"),
    ),
    ("contrast", include_str!("../../themes/contrast.toml")),
    ("daylight", include_str!("../../themes/daylight.toml")),
    ("sage", include_str!("../../themes/sage.toml")),
];
