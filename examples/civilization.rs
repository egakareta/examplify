//! A zero-player civilization sim told entirely through log messages.
//!
//! Run this in
//! - native `cargo run --example civilization`
//! - web `trunk serve --example civilization`

use game_loop::{Time, TimeTrait, game_loop};

const TECHNOLOGIES: [(&str, u64); 5] = [
    ("cultivation", 20),
    ("bronze tools", 75),
    ("written records", 180),
    ("irrigation", 400),
    ("public works", 800),
];

struct Civilization {
    year: u64,
    population: u32,
    food: f64,
    knowledge: u64,
    technologies: usize,
    next_population_milestone: u32,
    rng: fastrand::Rng,
}

impl Civilization {
    fn new() -> Self {
        Self {
            year: 0,
            population: 18,
            food: 54.0,
            knowledge: 0,
            technologies: 0,
            next_population_milestone: 25,
            rng: fastrand::Rng::new(),
        }
    }

    fn era(&self) -> &'static str {
        match self.technologies {
            0 => "Hearth Age",
            1 => "Cultivation Age",
            2 => "Bronze Age",
            3 => "Written Age",
            4 => "Irrigation Age",
            _ => "Civic Age",
        }
    }

    fn advance_year(&mut self) {
        self.year += 1;

        let harvest_per_person = 1.1 + self.technologies as f64 * 0.15;
        self.food += f64::from(self.population) * harvest_per_person;
        self.food -= f64::from(self.population);
        self.food = self.food.min(f64::from(self.population) * 12.0 + 100.0);

        let weather = self.rng.f64();
        if weather < 0.04 {
            self.food *= 0.45;
            log::warn!(
                "Year {}: drought scorches the fields. The stores fall to {:.0} meals.",
                self.year,
                self.food
            );
        } else if weather < 0.10 {
            self.food += f64::from(self.population) * 2.0;
            log::info!(
                "Year {}: an abundant harvest fills the granaries.",
                self.year
            );
        }

        self.knowledge += 1 + u64::from(self.population / 8) + self.technologies as u64;
        self.discover_technology();
        self.resolve_population();

        if self.year % 10 == 0 {
            log::info!(
                "Year {} | {}: {} people, {:.0} meals stored, {} knowledge.",
                self.year,
                self.era(),
                self.population,
                self.food,
                self.knowledge
            );
        }
    }

    fn discover_technology(&mut self) {
        let Some((name, threshold)) = TECHNOLOGIES.get(self.technologies).copied() else {
            return;
        };

        if self.knowledge >= threshold {
            self.technologies += 1;
            log::info!(
                "Year {}: discovery! The people master {}. A new age begins: {}.",
                self.year,
                name,
                self.era()
            );
        }
    }

    fn resolve_population(&mut self) {
        if self.food > f64::from(self.population) * 3.0 && self.rng.f64() < 0.35 {
            self.population += 1 + u32::from(self.rng.f64() < 0.2);
        } else if self.food < f64::from(self.population) * 0.75 && self.population > 1 {
            self.population -= 1;
            log::warn!(
                "Year {}: hunger takes a life. {} people remain.",
                self.year,
                self.population
            );
        }

        if self.rng.f64() < 0.025 {
            let newcomers = 1 + (self.rng.f64() * 3.0) as u32;
            self.population += newcomers;
            log::info!(
                "Year {}: a group of {} travelers joins the settlement.",
                self.year,
                newcomers
            );
        }

        if self.population >= self.next_population_milestone {
            log::info!(
                "Year {}: the settlement reaches {} people. Its old hearth cannot hold them all.",
                self.year,
                self.population
            );
            self.next_population_milestone *= 2;
        }
    }
}

fn main() {
    examplify::init().with_log_level(examplify::log::LevelFilter::Info);
    log::info!("A new settlement gathers around a small hearth.");

    game_loop(
        Civilization::new(),
        1,
        1.0,
        |game| game.game.advance_year(),
        |_| {
            if Time::supports_sleep() {
                Time::sleep(1.0 / 60.0);
            }
        },
    );
}
