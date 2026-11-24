//! The menu, and what a drink comes to with the chosen options: its layers, price and a
//! line describing it.

use crate::cup::{Liquid, Milk};

pub struct Drink {
    pub name: &'static str,
    pub note: &'static str,
    pub price: f64,
    /// Bottom to top: each liquid and how much of a medium cup it fills.
    pub layers: &'static [(Liquid, f64)],
}

use Liquid::*;

pub const MENU: [Drink; 8] = [
    Drink {
        name: "Espresso",
        note: "short and intense",
        price: 2.20,
        layers: &[(Espresso, 0.28), (Crema, 0.06)],
    },
    Drink {
        name: "Doppio",
        note: "a double shot",
        price: 2.80,
        layers: &[(Espresso, 0.46), (Crema, 0.07)],
    },
    Drink {
        name: "Americano",
        note: "espresso, hot water",
        price: 2.90,
        layers: &[(Water, 0.78), (Crema, 0.05)],
    },
    Drink {
        name: "Cappuccino",
        note: "espresso, milk, foam",
        price: 3.40,
        layers: &[(Espresso, 0.28), (Milk, 0.30), (Foam, 0.30)],
    },
    Drink {
        name: "Latte",
        note: "lots of milk",
        price: 3.60,
        layers: &[(Espresso, 0.22), (Milk, 0.56), (Foam, 0.12)],
    },
    Drink {
        name: "Flat White",
        note: "velvety microfoam",
        price: 3.50,
        layers: &[(Espresso, 0.32), (Milk, 0.44), (Foam, 0.07)],
    },
    Drink {
        name: "Macchiato",
        note: "espresso, a cap of foam",
        price: 2.70,
        layers: &[(Espresso, 0.32), (Foam, 0.16)],
    },
    Drink {
        name: "Mocha",
        note: "chocolate, espresso, cream",
        price: 3.90,
        layers: &[
            (Chocolate, 0.16),
            (Espresso, 0.24),
            (Milk, 0.32),
            (Cream, 0.16),
        ],
    },
];

pub const SIZES: [&str; 3] = ["Small", "Medium", "Large"];

/// What the person chose.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Options {
    /// 0 small, 1 medium, 2 large.
    pub size: usize,
    /// 1 mild … 5 very strong.
    pub strength: u8,
    pub milk: Milk,
    /// °C.
    pub temperature: i32,
    pub sugar: u8,
    pub extra_shot: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            size: 1,
            strength: 3,
            milk: Milk::Whole,
            temperature: 72,
            sugar: 0,
            extra_shot: false,
        }
    }
}

/// The drink's layers with these options: an extra shot, no milk (hot water instead),
/// and the size, scaled to fit the cup.
pub fn recipe(d: &Drink, o: &Options) -> Vec<(Liquid, f64)> {
    let mut layers: Vec<(Liquid, f64)> = Vec::new();
    for &(l, f) in d.layers {
        match (l, o.milk) {
            (Foam, Milk::None) => {}
            (Milk, Milk::None) => layers.push((Water, f)),
            _ => layers.push((l, f)),
        }
    }
    if o.extra_shot
        && let Some(coffee) = layers.iter_mut().find(|l| matches!(l.0, Espresso | Water))
    {
        coffee.1 += 0.1;
    }
    let scale = [0.78, 1.0, 1.14][o.size.min(2)];
    let total: f64 = layers.iter().map(|l| l.1).sum::<f64>() * scale;
    let fit = if total > 0.96 { 0.96 / total } else { 1.0 };
    layers.iter().map(|&(l, f)| (l, f * scale * fit)).collect()
}

/// The layers' paths and colours with `progress` (0–1) of the drink poured.
pub fn layers(
    recipe: &[(Liquid, f64)],
    progress: f64,
    o: &Options,
    phase: f64,
) -> Vec<(String, [u8; 3])> {
    let total: f64 = recipe.iter().map(|l| l.1).sum();
    let poured = total * progress.clamp(0.0, 1.0);
    let mut acc = 0.0;
    let mut out = Vec::new();
    for &(l, f) in recipe {
        let to = (acc + f).min(poured);
        if to > acc {
            out.push((
                crate::cup::layer(acc, to, l.wavy(), phase),
                l.color(o.strength, o.milk),
            ));
        }
        acc += f;
    }
    out
}

/// What's being poured at `progress`, for the status line and the stream's colour.
pub fn pouring(recipe: &[(Liquid, f64)], progress: f64) -> Option<Liquid> {
    let total: f64 = recipe.iter().map(|l| l.1).sum();
    let mut acc = 0.0;
    for &(l, f) in recipe {
        acc += f;
        if progress * total < acc {
            return Some(l);
        }
    }
    None
}

pub fn step_name(l: Liquid) -> &'static str {
    match l {
        Espresso => "Pulling the shot",
        Crema => "Crema forming",
        Water => "Adding hot water",
        Milk => "Steaming the milk",
        Foam => "Spooning the foam",
        Chocolate => "Melting chocolate",
        Cream => "Topping with cream",
    }
}

pub fn price(d: &Drink, o: &Options) -> f64 {
    let size = [-0.40, 0.0, 0.60][o.size.min(2)];
    let shot = if o.extra_shot { 0.60 } else { 0.0 };
    let milk =
        if matches!(o.milk, Milk::Oat | Milk::Almond) && d.layers.iter().any(|l| l.0.is_milk()) {
            0.40
        } else {
            0.0
        };
    d.price + size + shot + milk
}

pub fn money(v: f64) -> String {
    format!("€{v:.2}")
}

const STRENGTH: [&str; 5] = ["mild", "smooth", "balanced", "strong", "very strong"];

/// "Medium cappuccino · strong · oat milk · 72°C · 1 sugar".
pub fn summary(d: &Drink, o: &Options) -> String {
    let mut parts = vec![
        format!("{} {}", SIZES[o.size.min(2)], d.name.to_lowercase()),
        STRENGTH[(o.strength.clamp(1, 5) - 1) as usize].to_owned(),
    ];
    if d.layers.iter().any(|l| l.0.is_milk()) {
        parts.push(o.milk.name().to_owned());
    }
    parts.push(format!("{}°C", o.temperature));
    if o.extra_shot {
        parts.push("extra shot".into());
    }
    match o.sugar {
        0 => {}
        1 => parts.push("1 sugar".into()),
        n => parts.push(format!("{n} sugars")),
    }
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn by_name(n: &str) -> &'static Drink {
        MENU.iter().find(|d| d.name == n).unwrap()
    }

    #[test]
    fn options_change_the_recipe() {
        let cap = by_name("Cappuccino");
        let o = Options::default();
        assert_eq!(recipe(cap, &o).len(), 3);
        let black = recipe(
            cap,
            &Options {
                milk: Milk::None,
                ..o
            },
        );
        assert_eq!(
            black.iter().map(|l| l.0).collect::<Vec<_>>(),
            [Espresso, Water]
        );
        let big = recipe(cap, &Options { size: 2, ..o });
        let total: f64 = big.iter().map(|l| l.1).sum();
        assert!(total <= 0.961, "{total}");
        let shot = recipe(
            by_name("Espresso"),
            &Options {
                extra_shot: true,
                ..o
            },
        );
        assert!((shot[0].1 - 0.38).abs() < 1e-9, "{shot:?}");
    }

    #[test]
    fn pouring_goes_layer_by_layer() {
        let r = recipe(by_name("Latte"), &Options::default());
        assert_eq!(pouring(&r, 0.05), Some(Espresso));
        assert_eq!(pouring(&r, 0.6), Some(Milk));
        assert_eq!(pouring(&r, 1.0), None);
        assert_eq!(layers(&r, 0.0, &Options::default(), 0.0).len(), 0);
        assert_eq!(layers(&r, 1.0, &Options::default(), 0.0).len(), 3);
    }

    #[test]
    fn price_and_summary() {
        let cap = by_name("Cappuccino");
        let o = Options {
            size: 2,
            milk: Milk::Oat,
            strength: 4,
            sugar: 1,
            ..Default::default()
        };
        assert_eq!(money(price(cap, &o)), "€4.40");
        assert_eq!(
            summary(cap, &o),
            "Large cappuccino · strong · oat milk · 72°C · 1 sugar"
        );
        // Milk doesn't come into an espresso.
        assert!(!summary(by_name("Espresso"), &o).contains("milk"));
        assert_eq!(money(price(by_name("Espresso"), &o)), "€2.80");
    }
}
