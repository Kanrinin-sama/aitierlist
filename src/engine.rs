use crate::settings::Settings;
use crate::types::{CacheState, CandidatePick, Pick, Row, Seat, SeatTierPick, Table, Tier};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

const VENDORS: [&str; 14] = [
    "openai",
    "anthropic",
    "google",
    "xai",
    "cursor",
    "cursor-api",
    "moonshot",
    "alibaba",
    "zai",
    "minimax",
    "opencode",
    "cognition",
    "muse",
    "deepseek",
];

#[derive(Clone, Copy)]
pub enum Allowance {
    Usd(f64, f64),
    Calls(f64),
}

pub fn plan_for(vendor: &str, budget: f64) -> Option<Allowance> {
    use Allowance::{Calls, Usd};
    let plans: &[(f64, Allowance)] = match vendor {
        "openai" => &[
            (200.0, Usd(1538.0, 2272.0)),
            (100.0, Usd(384.0, 568.0)),
            (20.0, Usd(77.0, 114.0)),
        ],
        "anthropic" => &[
            (200.0, Usd(1100.0, 1300.0)),
            (100.0, Usd(500.0, 650.0)),
            (20.0, Usd(100.0, 130.0)),
        ],
        "google" => &[
            (199.99, Usd(19.99 * 20.0, 19.99 * 20.0)),
            (99.99, Usd(19.99 * 5.0, 19.99 * 5.0)),
            (19.99, Usd(19.99, 19.99)),
        ],
        "muse" => &[
            (50.0, Usd(50.0, 50.0)),
            (15.0, Usd(15.0, 15.0)),
            (5.0, Usd(5.0, 5.0)),
        ],
        "xai" => &[
            (300.0, Usd(300.0, 300.0)),
            (100.0, Usd(100.0, 100.0)),
            (30.0, Usd(19.0, 115.0)),
        ],
        "cursor" => &[
            (200.0, Usd(228.0, 228.0)),
            (60.0, Usd(34.0, 92.0)),
            (20.0, Usd(11.0, 11.0)),
        ],
        "cursor-api" => &[
            (200.0, Usd(92.31, 92.31)),
            (60.0, Usd(16.15, 16.15)),
            (20.0, Usd(4.62, 4.62)),
        ],
        "moonshot" => &[
            (199.0, Usd(200.0, 200.0)),
            (99.0, Usd(100.0, 100.0)),
            (39.0, Usd(33.0, 33.0)),
            (19.0, Usd(6.7, 6.7)),
        ],
        "alibaba" => &[(50.0, Calls(90000.0))],
        "zai" => &[
            (168.0, Usd(428.0, 568.0)),
            (80.0, Usd(184.0, 243.0)),
            (18.0, Usd(31.0, 41.0)),
        ],
        "minimax" => &[
            (132.0, Usd(132.0, 132.0)),
            (55.0, Usd(55.0, 55.0)),
            (22.0, Usd(22.0, 22.0)),
        ],
        "opencode" => &[(10.0, Usd(60.0, 60.0))],
        "cognition" => &[(200.0, Usd(200.0, 200.0)), (20.0, Usd(20.0, 20.0))],
        _ => &[],
    };
    plans
        .iter()
        .find(|(price, _)| *price <= budget)
        .map(|(_, allowance)| *allowance)
}

fn plan_eligible(row: &Row, plan: Option<Allowance>) -> bool {
    let Some(plan) = plan else { return false };
    if matches!(plan, Allowance::Calls(_))
        && !row
            .usd_per_step
            .is_some_and(|amount| amount > 0.0 && amount.is_finite())
    {
        return false;
    }
    !row.harness.eq_ignore_ascii_case("Claude Code")
        || !["GLM-5.1", "GLM-5.2", "Qwen3.8 Max"].iter().any(|model| {
            crate::aa::family_key("", &row.model_key)
                == crate::aa::family_key("", &crate::aa::harness_key(model))
        })
}

struct Mulberry32(u32);

impl Mulberry32 {
    fn random(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x6d2b79f5);
        let mut value = (self.0 ^ (self.0 >> 15)).wrapping_mul(1 | self.0);
        value = value.wrapping_add((value ^ (value >> 7)).wrapping_mul(61 | value)) ^ value;
        f64::from(value ^ (value >> 14)) / 4294967296.0
    }

    fn normal(&mut self) -> f64 {
        let mut first = self.random();
        while first == 0.0 {
            first = self.random();
        }
        let mut second = self.random();
        while second == 0.0 {
            second = self.random();
        }
        (-2.0 * first.ln()).sqrt() * (2.0 * std::f64::consts::PI * second).cos()
    }

    fn sample(&mut self, mean: f64, deviation: f64, accepts: impl Fn(f64) -> bool) -> f64 {
        if deviation <= 0.0 {
            return mean;
        }
        loop {
            let value = mean + deviation * self.normal();
            if accepts(value) {
                return value;
            }
        }
    }
}

fn quality(row: &Row, seat: Seat) -> Option<(f64, f64)> {
    let band = |key: &str| row.band.get(key).copied().flatten();
    let qna = row.qna.zip(band("sanityQuality"));
    let logic = row.logic.zip(band("logic"));
    let pass = Some(row.pass).zip(band("pass"));
    let average = |left: Option<(f64, f64)>, right: Option<(f64, f64)>| {
        left.zip(right)
            .map(|((a, da), (b, db))| ((a + b) / 2.0, (da / 2.0).hypot(db / 2.0)))
    };
    match seat {
        Seat::Implementer => pass,
        Seat::Debugger => average(pass, logic),
        Seat::Reviewer => average(qna, logic),
        Seat::Orchestrator => average(row.smart.zip(band("index")), row.gpqa.zip(band("gpqa"))),
        Seat::Sanity => qna,
        Seat::Comprehension => average(row.lcr.zip(band("lcr")), qna),
    }
    .filter(|(mean, deviation)| mean.is_finite() && deviation.is_finite())
}

fn basis(row: &Row, seat: Seat) -> (f64, f64, f64, f64) {
    if matches!(seat, Seat::Implementer | Seat::Debugger) {
        (
            row.wait_seconds,
            row.wait_seconds_band,
            row.attempt_usd,
            row.attempt_usd_band,
        )
    } else {
        (
            row.read_seconds,
            row.read_seconds_band,
            row.read_usd,
            row.read_usd_band,
        )
    }
}

pub struct Cycle {
    pub wall: f64,
    pub spend: f64,
}

pub fn cycle(
    quality: f64,
    seconds: u32,
    cost: f64,
    hallucination: f64,
    settings: &Settings,
) -> Cycle {
    let escalation_seconds = (settings.escalation_minutes * 60.0).round() as u32;
    let escalation = f64::from(escalation_seconds);
    let rho_ratio = settings.rho / (1.0 - settings.rho);
    let mut reached = 1.0;
    let mut attempts = 0.0;
    let mut escaped = 0.0;
    for attempt in 1..=64 {
        attempts += reached;
        let failed = reached * (1.0 - quality / (1.0 + f64::from(attempt - 1) * rho_ratio));
        escaped += failed * hallucination;
        reached = failed * (1.0 - hallucination);
        if quality / (1.0 + f64::from(attempt) * rho_ratio) * escalation <= f64::from(seconds) {
            break;
        }
    }
    let unfinished = escaped + reached;
    Cycle {
        wall: attempts * f64::from(seconds) + unfinished * escalation,
        spend: attempts * cost + unfinished * settings.escalation_usd,
    }
}

#[derive(Default)]
struct Total {
    tasks: f64,
    wall: f64,
    spend: f64,
    streams: f64,
    a_star_hours: f64,
    wins: u32,
}

fn simulate_pick(rows: &[Row], seat: Seat, tier: Tier, settings: &Settings) -> Option<Pick> {
    let budget = match tier {
        Tier::Api => -1.0,
        Tier::T200 => settings.plan_prices.t200,
        Tier::T100 => settings.plan_prices.t100,
        Tier::T20 => settings.plan_prices.t20,
    };
    let plans = VENDORS.map(|vendor| plan_for(vendor, budget));
    let candidates: Vec<_> = rows
        .iter()
        .enumerate()
        .filter_map(|(index, row)| {
            let vendor = VENDORS.iter().position(|vendor| *vendor == row.vendor);
            let plan = vendor.and_then(|vendor| plans[vendor]);
            let override_amount = settings
                .vendor_overrides
                .get(&row.vendor)
                .copied()
                .flatten();
            let (seconds, seconds_band, cost, cost_band) = basis(row, seat);
            let (mean, deviation) = quality(row, seat)?;
            (row.harness != "model"
                && (tier == Tier::Api
                    || plan_eligible(row, plan)
                    || (plan.is_none() && override_amount.is_some()))
                && seconds.is_finite()
                && seconds > 0.0
                && cost.is_finite()
                && cost > 0.0)
                .then_some((
                    index,
                    vendor,
                    override_amount,
                    mean,
                    deviation,
                    seconds,
                    seconds_band,
                    cost,
                    cost_band,
                ))
        })
        .collect();
    if candidates.is_empty() {
        return None;
    }
    let mut totals: Vec<Total> = candidates.iter().map(|_| Total::default()).collect();
    let mut random = Mulberry32(0x5eed);
    let draws = settings.draws.max(1);
    for _ in 0..draws {
        let allowance = plans.map(|plan| {
            (match plan {
                Some(Allowance::Usd(low, high)) if low != high => {
                    low + random.random() * (high - low)
                }
                Some(Allowance::Usd(amount, _)) | Some(Allowance::Calls(amount)) => amount,
                None => 0.0,
            }) * 12.0
                / 52.0
        });
        let mut winner = 0;
        let mut best_tasks = f64::NEG_INFINITY;
        for (
            position,
            &(
                index,
                vendor,
                override_amount,
                mean,
                deviation,
                seconds,
                seconds_band,
                cost,
                cost_band,
            ),
        ) in candidates.iter().enumerate()
        {
            let row = &rows[index];
            let sampled_quality =
                random.sample(mean, deviation, |value| (0.0..=1.0).contains(&value));
            let sampled_seconds = random
                .sample(seconds, seconds_band, |value| value > 0.0)
                .round()
                .max(1.0) as u32;
            let sampled_cost = random.sample(cost, cost_band, |value| value > 0.0);
            let hallucination = if seat == Seat::Implementer {
                0.0
            } else {
                row.halluc.unwrap_or(0.0)
            };
            let result = cycle(
                sampled_quality,
                sampled_seconds,
                sampled_cost,
                hallucination,
                settings,
            );
            let amount = override_amount.unwrap_or_else(|| {
                vendor
                    .map(|vendor| {
                        allowance[vendor]
                            * if matches!(plans[vendor], Some(Allowance::Calls(_))) {
                                row.usd_per_step.unwrap_or(0.0)
                            } else {
                                1.0
                            }
                    })
                    .unwrap_or(0.0)
            });
            let quota = if tier == Tier::Api {
                f64::INFINITY
            } else {
                amount / result.spend
            };
            let time = settings.agent_hours * 3600.0 / result.wall;
            let tasks = time.min(quota);
            let total = &mut totals[position];
            total.tasks += tasks;
            total.wall += result.wall;
            total.spend += result.spend;
            total.streams += quota / time;
            total.a_star_hours += quota * result.wall / 3600.0;
            if tasks > best_tasks {
                best_tasks = tasks;
                winner = position;
            }
        }
        totals[winner].wins += 1;
    }
    let count = f64::from(draws);
    let mut top: Vec<_> = candidates
        .iter()
        .zip(&totals)
        .map(|(candidate, total)| {
            let family = &rows[candidate.0].family;
            let family_totals: Vec<_> = candidates
                .iter()
                .zip(&totals)
                .filter(|(other, _)| rows[other.0].family == *family)
                .collect();
            let streams_star = (tier != Tier::Api).then_some(total.streams / count);
            let a_star_hours = (tier != Tier::Api).then_some(total.a_star_hours / count);
            CandidatePick {
                row_index: candidate.0,
                win_rate: f64::from(total.wins) / count,
                family_win_rate: family_totals
                    .iter()
                    .map(|(_, total)| f64::from(total.wins))
                    .sum::<f64>()
                    / count,
                n: family_totals.len(),
                minutes_per_task: total.wall / count / 60.0,
                cost_per_task: total.spend / count,
                tasks_per_week: total.tasks / count,
                streams_star,
                a_star_hours,
                util_pct: a_star_hours.map(|hours| 100.0 * (settings.agent_hours / hours).min(1.0)),
            }
        })
        .collect();
    top.sort_by(|left, right| {
        if tier == Tier::Api {
            right.tasks_per_week.total_cmp(&left.tasks_per_week)
        } else {
            right
                .win_rate
                .total_cmp(&left.win_rate)
                .then_with(|| right.tasks_per_week.total_cmp(&left.tasks_per_week))
        }
    });
    let first = &top[0];
    Some(Pick {
        row_index: first.row_index,
        win_rate: first.win_rate,
        family_win_rate: first.family_win_rate,
        n: first.n,
        minutes_per_task: first.minutes_per_task,
        cost_per_task: first.cost_per_task,
        tasks_per_week: first.tasks_per_week,
        streams_star: first.streams_star,
        a_star_hours: first.a_star_hours,
        util_pct: first.util_pct,
        top,
    })
}

pub fn score(
    mut rows: Vec<Row>,
    settings: &Settings,
    cache_state: CacheState,
    source_fetched_at: String,
) -> Table {
    rows.sort_by(|left, right| {
        let wall = |row: &Row| {
            cycle(
                row.pass,
                row.wait_seconds.round().max(1.0) as u32,
                row.attempt_usd,
                0.0,
                settings,
            )
            .wall
        };
        wall(left).total_cmp(&wall(right))
    });
    let picks = Seat::ALL
        .into_iter()
        .flat_map(|seat| Tier::ALL.into_iter().map(move |tier| (seat, tier)))
        .map(|(seat, tier)| SeatTierPick {
            seat,
            tier,
            pick: simulate_pick(&rows, seat, tier, settings),
        })
        .collect();
    Table {
        picks,
        rows,
        generated_at: OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .unwrap_or_default(),
        source_fetched_at,
        cache_state,
    }
}
