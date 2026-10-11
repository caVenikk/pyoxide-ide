//! `pyoxide-ide`: the Pyoxide Python IDE. Composes the platform workbench
//! with what is specific to Python (file type icons, tool windows, the
//! interpreter widget).

use ui::icons::{IconName, IconSvg};
use workbench::{Product, StripeItem};

/// Icons of the Python product (IntelliJ Platform Community, Apache-2.0,
/// see NOTICE).
const ICONS: &[IconSvg] = &[
    IconSvg {
        name: "python/file",
        light: include_bytes!("../assets/icons/python.svg"),
        dark: Some(include_bytes!("../assets/icons/python_dark.svg")),
    },
    IconSvg {
        name: "python/packages",
        light: include_bytes!("../assets/icons/pythonPackages@20x20.svg"),
        dark: Some(include_bytes!(
            "../assets/icons/pythonPackages@20x20_dark.svg"
        )),
    },
    IconSvg {
        name: "python/console",
        light: include_bytes!("../assets/icons/pythonConsoleToolWindow@20x20.svg"),
        dark: Some(include_bytes!(
            "../assets/icons/pythonConsoleToolWindow@20x20_dark.svg"
        )),
    },
];

fn product() -> Product {
    Product {
        name: "Pyoxide",
        icons: ICONS,
        left_stripe_bottom: vec![
            StripeItem {
                id: "stripe-python-packages",
                icon: IconName("python/packages"),
                title: "Python Packages",
            },
            StripeItem {
                id: "stripe-python-console",
                icon: IconName("python/console"),
                title: "Python Console",
            },
        ],
        interpreter_label: Some("No interpreter"),
    }
}

fn main() {
    let paths = std::env::args_os().skip(1).map(Into::into).collect();
    workbench::run(product(), paths);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn product_icons_rasterize() {
        for icon in ICONS {
            assert!(
                ui::icons::rasterize(icon.light, 32).is_some(),
                "{}",
                icon.name
            );
            assert!(
                ui::icons::rasterize(icon.dark.unwrap(), 32).is_some(),
                "{}",
                icon.name
            );
        }
        assert_eq!(product().left_stripe_bottom.len(), 2);
    }
}
