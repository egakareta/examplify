//! A zero-player civilization sim told entirely through log messages.
//!
//! Run this in
//! - native `cargo run --example civilization`
//! - web `trunk serve --example civilization`

use game_loop::{Time, TimeTrait, game_loop};

const TECHNOLOGIES: [(&str, u64); 10] = [
    ("cultivation", 20),
    ("bronze tools", 75),
    ("written records", 180),
    ("irrigation", 400),
    ("public works", 800),
    ("astronomy", 1_400),
    ("civic law", 2_200),
    ("navigation", 3_300),
    ("medicine", 4_700),
    ("mechanical craft", 6_400),
];

struct Civilization {
    year: u64,
    population: u32,
    food: f64,
    water: f64,
    knowledge: u64,
    technologies: usize,
    next_population_milestone: u32,
    timber: f64,
    stone: f64,
    ore: f64,
    wealth: f64,
    forest: f64,
    morale: f64,
    health: f64,
    stability: f64,
    culture: f64,
    pollution: f64,
    farms: u32,
    housing: u32,
    wells: u32,
    granaries: u32,
    markets: u32,
    clinics: u32,
    walls: u32,
    schools: u32,
    smithies: u32,
    rng: fastrand::Rng,
}

impl Civilization {
    fn new() -> Self {
        Self {
            year: 0,
            population: 18,
            food: 54.0,
            water: 36.0,
            knowledge: 0,
            technologies: 0,
            next_population_milestone: 25,
            timber: 35.0,
            stone: 20.0,
            ore: 0.0,
            wealth: 8.0,
            forest: 400.0,
            morale: 68.0,
            health: 90.0,
            stability: 65.0,
            culture: 5.0,
            pollution: 0.0,
            farms: 2,
            housing: 24,
            wells: 1,
            granaries: 0,
            markets: 0,
            clinics: 0,
            walls: 0,
            schools: 0,
            smithies: 0,
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
            5 => "Civic Age",
            6 => "Star Age",
            7 => "Law Age",
            8 => "Sail Age",
            9 => "Medicine Age",
            _ => "Mechanical Age",
        }
    }

    fn advance_year(&mut self) {
        self.year = self.year.saturating_add(1);

        let season = ((self.year - 1) % 4) as usize;
        let season_name = ["winter", "spring", "summer", "autumn"][season];
        let season_harvest = [0.72, 0.92, 1.20, 1.08][season];
        let rainfall = [0.68, 1.15, 0.52, 0.78][season];

        self.produce_food(season_harvest, rainfall);
        self.gather_resources();
        self.resolve_event();
        self.resolve_trade();
        self.resolve_consumption();
        self.resolve_health_and_morale();
        self.advance_knowledge();
        self.discover_technology();
        self.build_infrastructure();
        self.resolve_population();
        self.resolve_culture_and_unrest();

        if self.year % 4 == 1 {
            log::debug!("Year {} begins in {}.", self.year, season_name);
        }

        if self.year % 10 == 0 {
            log::info!(
                "Year {} | {}: {} people, {:.0} meals, {:.0} water, {:.0} timber, {:.0} stone, {:.0} ore, {:.0} wealth, {} knowledge. Morale {:.0}, health {:.0}, stability {:.0}.",
                self.year,
                self.era(),
                self.population,
                self.food,
                self.water,
                self.timber,
                self.stone,
                self.ore,
                self.wealth,
                self.knowledge,
                self.morale,
                self.health,
                self.stability
            );
        }
    }

    fn produce_food(&mut self, season_harvest: f64, rainfall: f64) {
        let people = f64::from(self.population);
        let rain = rainfall * if self.rng.f64() < 0.08 { 0.35 } else { 1.0 };
        let water_produced = people * rain * 0.55 + f64::from(self.wells) * people * 0.12;
        self.water += water_produced;

        let water_ratio = (self.water / (people * 0.45).max(1.0)).clamp(0.0, 1.25);
        let irrigation = if self.technologies >= 4 { 0.16 } else { 0.0 };
        let harvest_per_person =
            1.0 + self.technologies as f64 * 0.10 + f64::from(self.farms) * 0.035 + irrigation;
        let weather = self.rng.f64();
        let weather_harvest = if weather < 0.035 {
            self.food *= 0.72;
            self.water *= 0.82;
            log::warn!(
                "Year {}: drought dries the furrows and wells; stores fall to {:.0} meals.",
                self.year,
                self.food
            );
            0.45
        } else if weather < 0.09 {
            log::info!(
                "Year {}: gentle rains bring an abundant harvest.",
                self.year
            );
            1.45
        } else if weather < 0.12 {
            log::warn!("Year {}: hail batters the ripening crops.", self.year);
            0.65
        } else {
            1.0
        };

        self.food += people * harvest_per_person * season_harvest * weather_harvest * water_ratio;
        let food_capacity = people * 12.0 + 100.0 + f64::from(self.granaries) * 80.0;
        if self.food > food_capacity {
            self.food = food_capacity;
            log::debug!(
                "Year {}: surplus grain spoils beyond storage capacity.",
                self.year
            );
        }

        let irrigation_cost = people * irrigation * 0.8;
        self.water = (self.water - irrigation_cost).max(0.0);
    }

    fn gather_resources(&mut self) {
        let people = f64::from(self.population);
        let tool_bonus = 1.0 + f64::from(self.smithies) * 0.15;
        let timber_gathered = self.forest.min(people * 0.11 * tool_bonus);
        self.forest -= timber_gathered;
        self.forest = (self.forest + 2.0 + self.forest * 0.012).min(800.0);
        self.timber += timber_gathered;
        self.stone += people * (0.025 + self.technologies as f64 * 0.002) * tool_bonus;
        self.ore += people * (0.012 + if self.technologies >= 2 { 0.018 } else { 0.0 });

        if self.forest < 35.0 && self.year % 5 == 0 {
            log::warn!(
                "Year {}: the nearby forest is nearly exhausted; woodcutters range farther afield.",
                self.year
            );
        }
        self.pollution = (self.pollution + people * 0.008 - 0.4).clamp(0.0, 100.0);
    }

    fn resolve_event(&mut self) {
        // A single annual roll selects among 25 events; quiet years remain possible.
        match self.rng.u32(0..1000) {
            0..=19 => {
                self.food *= 0.68;
                self.morale -= 7.0;
                log::warn!("Year {}: a crop blight blackens the fields.", self.year);
            }
            20..=39 => {
                self.water *= 0.7;
                self.food += f64::from(self.population) * 0.25;
                log::warn!(
                    "Year {}: river flooding damages homes but leaves fertile silt.",
                    self.year
                );
            }
            40..=59 => {
                let loss = self.timber.min(12.0 + f64::from(self.population) * 0.2);
                self.timber -= loss;
                self.morale -= 5.0;
                log::warn!(
                    "Year {}: a wildfire consumes {:.0} timber from the stores.",
                    self.year,
                    loss
                );
            }
            60..=79 => {
                self.stone *= 0.8;
                self.stability -= 6.0;
                log::warn!(
                    "Year {}: an earthquake cracks walls and collapses storehouses.",
                    self.year
                );
            }
            80..=99 => {
                self.health -= 16.0;
                self.morale -= 8.0;
                log::error!(
                    "Year {}: fever spreads through the crowded settlement.",
                    self.year
                );
            }
            100..=119 => {
                self.health = (self.health + 9.0 + f64::from(self.clinics) * 3.0).min(100.0);
                self.morale += 3.0;
                log::info!(
                    "Year {}: traveling healers share remedies and tend the sick.",
                    self.year
                );
            }
            120..=139 => {
                self.wealth += 18.0 + f64::from(self.markets) * 8.0;
                self.timber += 5.0;
                log::info!(
                    "Year {}: a merchant caravan trades cloth and salt for local goods.",
                    self.year
                );
            }
            140..=159 => {
                self.food -= f64::from(self.population) * 0.35;
                self.morale += 12.0;
                self.culture += 4.0;
                log::info!(
                    "Year {}: a harvest festival lifts spirits and fills the common tables.",
                    self.year
                );
            }
            160..=179 => {
                self.knowledge += 24;
                self.ore += 8.0;
                log::info!(
                    "Year {}: a craftsperson invents a more reliable smelting method.",
                    self.year
                );
            }
            180..=199 => {
                let stolen = self.wealth.min(12.0 + f64::from(self.population) * 0.25);
                self.wealth -= stolen;
                self.food *= 0.92;
                self.stability -= 8.0 - f64::from(self.walls).min(6.0);
                log::warn!(
                    "Year {}: bandits raid the outskirts and escape with {:.0} wealth.",
                    self.year,
                    stolen
                );
            }
            200..=219 => {
                let newcomers = 2 + self.rng.u32(0..5);
                self.population = self.population.saturating_add(newcomers);
                self.morale += 4.0;
                log::info!(
                    "Year {}: {} displaced travelers seek refuge at the hearth.",
                    self.year,
                    newcomers
                );
            }
            220..=239 => {
                self.stability -= 12.0;
                self.morale -= 4.0;
                log::warn!(
                    "Year {}: a boundary dispute divides the elders' council.",
                    self.year
                );
            }
            240..=259 => {
                let departing = (self.population / 20)
                    .max(1)
                    .min(self.population.saturating_sub(1));
                self.population -= departing;
                self.wealth += f64::from(departing) * 0.6;
                log::warn!(
                    "Year {}: {} families leave to seek better farmland.",
                    self.year,
                    departing
                );
            }
            260..=279 => {
                self.food += f64::from(self.population) * 1.2;
                self.health += 4.0;
                log::info!(
                    "Year {}: wild herds pass nearby, providing meat and hides.",
                    self.year
                );
            }
            280..=299 => {
                self.ore += 20.0;
                self.wealth += 8.0;
                log::info!(
                    "Year {}: miners uncover a rich vein of copper ore.",
                    self.year
                );
            }
            300..=319 => {
                self.water += f64::from(self.population) * 2.0;
                self.health += 3.0;
                log::info!(
                    "Year {}: a spring is found beneath the eastern ridge.",
                    self.year
                );
            }
            320..=339 => {
                self.food -= f64::from(self.population) * 0.5;
                self.morale += 8.0;
                self.stability += 3.0;
                log::info!(
                    "Year {}: the elders host a communal feast to settle old grievances.",
                    self.year
                );
            }
            340..=359 => {
                self.food *= 0.88;
                self.health -= 4.0;
                log::warn!(
                    "Year {}: rats get into the grain stores and spread sickness.",
                    self.year
                );
            }
            360..=379 => {
                self.stability += 10.0;
                self.morale += 5.0;
                log::info!(
                    "Year {}: a respected steward restores trust in the council.",
                    self.year
                );
            }
            380..=399 => {
                self.knowledge += 16;
                self.wealth += 10.0;
                log::info!(
                    "Year {}: envoys arrive bearing maps and news from distant settlements.",
                    self.year
                );
            }
            400..=419 => {
                self.culture += 8.0;
                self.morale += 4.0;
                log::info!(
                    "Year {}: pilgrims bring songs and stories from the wider world.",
                    self.year
                );
            }
            420..=439 => {
                self.health -= 5.0;
                self.timber -= self.timber.min(8.0);
                log::warn!(
                    "Year {}: a workshop accident injures workers and damages tools.",
                    self.year
                );
            }
            440..=459 => {
                self.wealth -= self.wealth.min(8.0);
                self.stability += 4.0;
                log::info!(
                    "Year {}: neighbors help rebuild the old road after a landslide.",
                    self.year
                );
            }
            460..=479 => {
                self.food -= f64::from(self.population) * 0.2;
                self.morale += 6.0;
                self.culture += 3.0;
                log::info!(
                    "Year {}: a storyteller's performance becomes a new local tradition.",
                    self.year
                );
            }
            480..=499 => {
                self.knowledge += 30;
                self.culture += 5.0;
                log::info!(
                    "Year {}: a cache of old tablets reveals forgotten histories.",
                    self.year
                );
            }
            _ => {}
        }
    }

    fn resolve_trade(&mut self) {
        if self.markets == 0 {
            return;
        }

        let food_ratio = self.food / f64::from(self.population).max(1.0);
        if food_ratio > 5.0 && self.wealth < 200.0 {
            let sold = self.food.min(f64::from(self.population) * 0.8);
            self.food -= sold;
            self.wealth += sold * (0.18 + f64::from(self.markets) * 0.03);
            if self.year % 7 == 0 {
                log::info!(
                    "Year {}: market traders exchange surplus grain for coins.",
                    self.year
                );
            }
        } else if food_ratio < 1.5 && self.wealth >= 10.0 {
            let bought = (self.wealth / 0.3).min(f64::from(self.population) * 2.0);
            self.food += bought;
            self.wealth -= bought * 0.3;
            log::info!(
                "Year {}: the market imports grain to ease a lean season.",
                self.year
            );
        }
    }

    fn resolve_consumption(&mut self) {
        let people = f64::from(self.population);
        let food_needed = people * (1.0 + if self.health < 35.0 { 0.1 } else { 0.0 });
        let water_needed = people * (0.42 + if self.technologies >= 4 { 0.08 } else { 0.0 });
        self.food -= food_needed;
        self.water -= water_needed;

        if self.food < 0.0 {
            self.morale -= 8.0;
            self.health -= 5.0;
            log::warn!(
                "Year {}: the harvest cannot feed everyone; hunger spreads.",
                self.year
            );
        }
        if self.water < 0.0 {
            self.morale -= 5.0;
            self.health -= 4.0;
            log::warn!(
                "Year {}: wells run low and families ration their water.",
                self.year
            );
        }
        self.food = self.food.max(0.0);
        self.water = self.water.max(0.0);
    }

    fn resolve_health_and_morale(&mut self) {
        let sanitation = f64::from(self.wells.min(self.clinics));
        let disease_pressure = (self.pollution * 0.08 + f64::from(self.population) / 100.0
            - sanitation * 1.5)
            .max(0.0);
        self.health += f64::from(self.clinics) * 1.4 + 0.8 - disease_pressure;
        if self.health < 25.0 && self.population > 1 && self.rng.f64() < 0.18 {
            self.population -= 1;
            log::error!(
                "Year {}: illness claims a life; {} people remain.",
                self.year,
                self.population
            );
        }

        let food_ratio = self.food / f64::from(self.population).max(1.0);
        let housing_ratio = f64::from(self.housing) / f64::from(self.population).max(1.0);
        self.morale += if food_ratio > 3.0 { 1.8 } else { -2.5 };
        self.morale += if housing_ratio >= 1.0 { 0.8 } else { -2.0 };
        self.morale += if self.water > f64::from(self.population) {
            0.5
        } else {
            -1.2
        };
        self.morale -= self.pollution * 0.015;
        self.health = self.health.clamp(0.0, 100.0);
        self.morale = self.morale.clamp(0.0, 100.0);
    }

    fn advance_knowledge(&mut self) {
        let earned = 1
            + u64::from(self.population / 8)
            + self.technologies as u64
            + u64::from(self.schools) * 3
            + (self.culture / 25.0) as u64;
        self.knowledge = self.knowledge.saturating_add(earned);
    }

    fn discover_technology(&mut self) {
        let Some((name, threshold)) = TECHNOLOGIES.get(self.technologies).copied() else {
            return;
        };

        if self.knowledge >= threshold {
            self.technologies += 1;
            self.morale += 3.0;
            log::info!(
                "Year {}: discovery! The people master {}. A new age begins: {}.",
                self.year,
                name,
                self.era()
            );
        }
    }

    fn build_infrastructure(&mut self) {
        if self.population > self.housing && self.can_afford(14.0, 4.0) {
            self.spend(14.0, 4.0);
            self.housing = self.housing.saturating_add(12);
            log::info!(
                "Year {}: builders raise new homes; shelter now holds {} people.",
                self.year,
                self.housing
            );
        } else if self.wells < (self.population / 24).max(1) && self.can_afford(9.0, 5.0) {
            self.spend(9.0, 5.0);
            self.wells += 1;
            log::info!(
                "Year {}: a stone-lined well improves the water supply.",
                self.year
            );
        } else if self.farms < (self.population / 10).max(2)
            && self.food < f64::from(self.population) * 8.0
            && self.can_afford(12.0, 2.0)
        {
            self.spend(12.0, 2.0);
            self.farms += 1;
            log::info!(
                "Year {}: another family clears land for cultivation.",
                self.year
            );
        } else if self.granaries < (self.population / 35).max(1) && self.can_afford(18.0, 5.0) {
            self.spend(18.0, 5.0);
            self.granaries += 1;
            log::info!("Year {}: a granary expands safe food storage.", self.year);
        } else if self.markets == 0 && self.technologies >= 2 && self.can_afford(20.0, 10.0) {
            self.spend(20.0, 10.0);
            self.markets += 1;
            log::info!(
                "Year {}: a market square opens, enabling long-distance trade.",
                self.year
            );
        } else if self.schools < (self.population / 60).max(1)
            && self.technologies >= 3
            && self.can_afford(16.0, 10.0)
        {
            self.spend(16.0, 10.0);
            self.schools += 1;
            log::info!(
                "Year {}: a school begins teaching the next generation.",
                self.year
            );
        } else if self.smithies < (self.population / 80).max(1)
            && self.technologies >= 2
            && self.ore >= 8.0
            && self.can_afford(14.0, 8.0)
        {
            self.spend(14.0, 8.0);
            self.ore -= 8.0;
            self.smithies += 1;
            log::info!(
                "Year {}: a smithy forges better tools from the settlement's copper.",
                self.year
            );
        } else if self.clinics == 0 && self.technologies >= 4 && self.can_afford(12.0, 18.0) {
            self.spend(12.0, 18.0);
            self.clinics += 1;
            log::info!("Year {}: the first clinic opens its doors.", self.year);
        } else if self.walls == 0 && self.technologies >= 5 && self.can_afford(8.0, 28.0) {
            self.spend(8.0, 28.0);
            self.walls += 1;
            log::info!(
                "Year {}: stone walls shelter the settlement from raiders.",
                self.year
            );
        }
    }

    fn can_afford(&self, timber: f64, stone: f64) -> bool {
        self.timber >= timber && self.stone >= stone
    }

    fn spend(&mut self, timber: f64, stone: f64) {
        self.timber -= timber;
        self.stone -= stone;
    }

    fn resolve_population(&mut self) {
        let people = f64::from(self.population);
        let food_per_person = self.food / people.max(1.0);
        let room_to_grow = self.population < self.housing;
        let growth_chance = (0.18 + self.morale / 250.0 + self.health / 500.0).clamp(0.0, 0.55);

        if food_per_person > 2.0
            && self.water > people * 0.3
            && room_to_grow
            && self.rng.f64() < growth_chance
        {
            let births = 1 + u32::from(self.morale > 75.0 && self.rng.f64() < 0.2);
            self.population = self.population.saturating_add(births);
            log::info!(
                "Year {}: {} new lives join the settlement.",
                self.year,
                births
            );
        } else if (food_per_person < 0.25 || self.health < 15.0) && self.population > 1 {
            self.population -= 1;
            log::warn!(
                "Year {}: hardship takes a life. {} people remain.",
                self.year,
                self.population
            );
        }

        if self.morale > 72.0 && self.rng.f64() < 0.018 {
            let newcomers = 1 + self.rng.u32(0..3);
            self.population = self.population.saturating_add(newcomers);
            log::info!(
                "Year {}: hopeful travelers bring {} newcomers.",
                self.year,
                newcomers
            );
        } else if self.morale < 18.0 && self.rng.f64() < 0.08 && self.population > 2 {
            self.population -= 1;
            log::warn!(
                "Year {}: one family abandons the troubled settlement.",
                self.year
            );
        }

        if self.population >= self.next_population_milestone {
            log::info!(
                "Year {}: the settlement reaches {} people. Its old hearth cannot hold them all.",
                self.year,
                self.population
            );
            self.next_population_milestone = self.next_population_milestone.saturating_mul(2);
        }
    }

    fn resolve_culture_and_unrest(&mut self) {
        self.culture = (self.culture
            + 0.15
            + f64::from(self.schools) * 0.25
            + if self.morale > 70.0 { 0.2 } else { 0.0 })
        .min(100.0);
        self.stability += if self.morale < 25.0 { -2.0 } else { 0.25 };
        if self.stability < 25.0 && self.rng.f64() < 0.12 {
            let lost = self.wealth.min(5.0 + f64::from(self.population) * 0.1);
            self.wealth -= lost;
            self.stability += 5.0;
            log::warn!(
                "Year {}: unrest disrupts the council; stores lose {:.0} wealth.",
                self.year,
                lost
            );
        }
        self.stability = self.stability.clamp(0.0, 100.0);
        self.wealth = self.wealth.max(0.0);
        self.food = self.food.max(0.0);
        self.water = self.water.max(0.0);
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
