#[derive(Clone)]
pub struct Choice {
    pub quality: f64,
    pub nominal_time: f64,
    pub nominal_cost: f64,
    pub resources: Vec<f64>,
}

pub struct Problem {
    pub groups: Vec<Vec<Choice>>,
    pub capacities: Vec<f64>,
}

pub struct Solution<T> {
    pub choices: Vec<usize>,
    pub quality: f64,
    pub nominal_time: f64,
    pub nominal_cost: f64,
    pub payload: T,
}

pub struct Outcome<T> {
    pub solution: Option<Solution<T>>,
    pub bound: Option<f64>,
    pub proven: bool,
    pub nodes: usize,
    pub limitation: Option<&'static str>,
}

struct Node {
    fixed: Vec<Option<usize>>,
    bound: f64,
}

struct Relaxation {
    fractions: Vec<Vec<f64>>,
    quality: f64,
    nominal_time: f64,
    nominal_cost: f64,
}

fn simplex(
    constraints: &[Vec<f64>],
    bounds: &[f64],
    objectives: &[Vec<f64>],
) -> Result<Vec<f64>, &'static str> {
    let variables = objectives[0].len();
    let count = constraints.len();
    let rhs = variables + count;
    let mut matrix = vec![vec![0.0; rhs + 1]; count + objectives.len()];
    for (index, (constraint, bound)) in constraints.iter().zip(bounds).enumerate() {
        let scale = constraint
            .iter()
            .map(|value| value.abs())
            .fold(bound.abs(), f64::max)
            .max(f64::MIN_POSITIVE);
        for (column, value) in constraint.iter().enumerate() {
            matrix[index][column] = value / scale;
        }
        matrix[index][variables + index] = 1.0;
        matrix[index][rhs] = bound / scale;
    }
    for (index, objective) in objectives.iter().enumerate() {
        let scale = objective
            .iter()
            .map(|value| value.abs())
            .fold(0.0, f64::max)
            .max(f64::MIN_POSITIVE);
        for (column, value) in objective.iter().enumerate() {
            matrix[count + index][column] = -value / scale;
        }
    }
    let mut basis: Vec<_> = (variables..rhs).collect();
    for _ in 0..20_000 {
        let entering = (count..matrix.len()).find_map(|objective| {
            (0..rhs).find(|column| {
                matrix[objective][*column] < -1e-10
                    && (count..objective).all(|higher| matrix[higher][*column].abs() <= 1e-10)
            })
        });
        let Some(entering) = entering else {
            let mut values = vec![0.0; variables];
            for (row, column) in basis.into_iter().enumerate() {
                if column < variables {
                    values[column] = matrix[row][rhs].max(0.0);
                }
            }
            return Ok(values);
        };
        let leaving = (0..count)
            .filter(|row| matrix[*row][entering] > 1e-12)
            .min_by(|left, right| {
                let primary = (matrix[*left][rhs] / matrix[*left][entering])
                    .total_cmp(&(matrix[*right][rhs] / matrix[*right][entering]));
                if !primary.is_eq() {
                    return primary;
                }
                for column in variables..rhs {
                    let left_value = matrix[*left][column] / matrix[*left][entering];
                    let right_value = matrix[*right][column] / matrix[*right][entering];
                    if (left_value - right_value).abs()
                        > 1e-12 * left_value.abs().max(right_value.abs()).max(1.0)
                    {
                        return left_value.total_cmp(&right_value);
                    }
                }
                basis[*left].cmp(&basis[*right])
            })
            .ok_or("linear relaxation has no stable bounded pivot")?;
        let divisor = matrix[leaving][entering];
        for value in &mut matrix[leaving] {
            *value /= divisor;
        }
        let pivot = matrix[leaving].clone();
        for (row_index, row) in matrix.iter_mut().enumerate() {
            if row_index != leaving {
                let factor = row[entering];
                for (value, pivot_value) in row.iter_mut().zip(&pivot) {
                    *value -= factor * pivot_value;
                    if !value.is_finite() {
                        return Err("linear relaxation produced a non-finite coefficient");
                    }
                }
                if row_index < count {
                    if row[rhs] < -1e-12 {
                        return Err("linear relaxation lost primal feasibility");
                    }
                    if row[rhs].abs() <= 1e-12 {
                        row[rhs] = 0.0;
                    }
                }
            }
        }
        basis[leaving] = entering;
    }
    Err("linear relaxation reached its pivot limit")
}

fn relax(
    problem: &Problem,
    fixed: &[Option<usize>],
    cuts: &[Vec<usize>],
) -> Result<Option<Relaxation>, &'static str> {
    let mut capacities = problem.capacities.clone();
    let mut fractions: Vec<_> = problem
        .groups
        .iter()
        .map(|group| vec![0.0; group.len()])
        .collect();
    let mut variables = Vec::new();
    for (group_index, group) in problem.groups.iter().enumerate() {
        if let Some(index) = fixed[group_index] {
            fractions[group_index][index] = 1.0;
            for (capacity, usage) in capacities.iter_mut().zip(&group[index].resources) {
                *capacity -= usage;
            }
        } else {
            variables.extend((0..group.len()).map(|index| (group_index, index)));
        }
    }
    if capacities.iter().any(|capacity| *capacity < -1e-8) {
        return Ok(None);
    }
    for capacity in &mut capacities {
        *capacity = capacity.max(0.0);
    }
    let mut constraints = Vec::new();
    let mut bounds = Vec::new();
    for (group, selection) in fixed.iter().enumerate() {
        if selection.is_none() {
            constraints.push(
                variables
                    .iter()
                    .map(|(candidate_group, _)| f64::from(*candidate_group == group))
                    .collect(),
            );
            bounds.push(1.0);
        }
    }
    let free_groups = bounds.len();
    for (resource, capacity) in capacities.iter().enumerate() {
        constraints.push(
            variables
                .iter()
                .map(|(group, choice)| problem.groups[*group][*choice].resources[resource])
                .collect(),
        );
        bounds.push(*capacity);
    }
    for cut in cuts {
        let fixed_matches = fixed
            .iter()
            .zip(cut)
            .filter(|(selected, excluded)| selected.is_some_and(|value| value == **excluded))
            .count();
        let bound = problem.groups.len() as f64 - 1.0 - fixed_matches as f64;
        if bound < 0.0 {
            return Ok(None);
        }
        constraints.push(
            variables
                .iter()
                .map(|(group, choice)| f64::from(cut[*group] == *choice))
                .collect(),
        );
        bounds.push(bound);
    }
    if !variables.is_empty() {
        let objectives = vec![
            vec![1.0; variables.len()],
            variables
                .iter()
                .map(|(group, choice)| problem.groups[*group][*choice].quality)
                .collect(),
            variables
                .iter()
                .map(|(group, choice)| -problem.groups[*group][*choice].nominal_time)
                .collect(),
            variables
                .iter()
                .map(|(group, choice)| -problem.groups[*group][*choice].nominal_cost)
                .collect(),
        ];
        let values = simplex(&constraints, &bounds, &objectives)?;
        if values.iter().sum::<f64>() < free_groups as f64 - 1e-7 {
            return Ok(None);
        }
        for ((group, choice), value) in variables.into_iter().zip(values) {
            fractions[group][choice] = value;
        }
    }
    let mut result = Relaxation {
        fractions,
        quality: 0.0,
        nominal_time: 0.0,
        nominal_cost: 0.0,
    };
    for (group, values) in problem.groups.iter().zip(&result.fractions) {
        for (choice, fraction) in group.iter().zip(values) {
            result.quality += fraction * choice.quality;
            result.nominal_time += fraction * choice.nominal_time;
            result.nominal_cost += fraction * choice.nominal_cost;
        }
    }
    Ok(Some(result))
}

fn better<T>(quality: f64, time: f64, cost: f64, incumbent: &Solution<T>) -> bool {
    quality > incumbent.quality + 1e-8
        || ((quality - incumbent.quality).abs() <= 1e-8
            && (time < incumbent.nominal_time - 1e-8
                || ((time - incumbent.nominal_time).abs() <= 1e-8
                    && cost < incumbent.nominal_cost - 1e-8)))
}

fn candidate<T>(
    problem: &Problem,
    choices: Vec<usize>,
    evaluate: &mut impl FnMut(&[usize]) -> Option<T>,
) -> Option<Solution<T>> {
    let mut resources = vec![0.0; problem.capacities.len()];
    let mut quality = 0.0;
    let mut nominal_time = 0.0;
    let mut nominal_cost = 0.0;
    for (group, index) in problem.groups.iter().zip(&choices) {
        let choice = &group[*index];
        for (total, usage) in resources.iter_mut().zip(&choice.resources) {
            *total += usage;
        }
        quality += choice.quality;
        nominal_time += choice.nominal_time;
        nominal_cost += choice.nominal_cost;
    }
    if resources
        .iter()
        .zip(&problem.capacities)
        .any(|(usage, capacity)| *usage > *capacity + 1e-8)
    {
        return None;
    }
    let payload = evaluate(&choices)?;
    Some(Solution {
        choices,
        quality,
        nominal_time,
        nominal_cost,
        payload,
    })
}

fn improve<T>(
    problem: &Problem,
    incumbent: &mut Option<Solution<T>>,
    evaluate: &mut impl FnMut(&[usize]) -> Option<T>,
) {
    for _ in 0..problem.groups.len().max(1) {
        let Some(current) = incumbent.as_ref() else {
            return;
        };
        let choices = current.choices.clone();
        let mut replacement = None;
        for (group, options) in problem.groups.iter().enumerate() {
            for choice in 0..options.len() {
                if choice == choices[group] {
                    continue;
                }
                let mut next = choices.clone();
                next[group] = choice;
                let Some(solution) = candidate(problem, next, evaluate) else {
                    continue;
                };
                if incumbent.as_ref().is_some_and(|best| {
                    better(
                        solution.quality,
                        solution.nominal_time,
                        solution.nominal_cost,
                        best,
                    )
                }) && replacement.as_ref().is_none_or(|best: &Solution<T>| {
                    better(
                        solution.quality,
                        solution.nominal_time,
                        solution.nominal_cost,
                        best,
                    )
                }) {
                    replacement = Some(solution);
                }
            }
        }
        let Some(solution) = replacement else {
            return;
        };
        *incumbent = Some(solution);
    }
}

fn resource_score(choice: &Choice, capacities: &[f64]) -> f64 {
    choice
        .resources
        .iter()
        .zip(capacities)
        .map(|(usage, capacity)| {
            if *usage == 0.0 {
                0.0
            } else if *capacity > 0.0 {
                usage / capacity
            } else {
                f64::INFINITY
            }
        })
        .sum()
}

fn repair(problem: &Problem, mut choices: Vec<usize>, fixed: &[Option<usize>]) -> Vec<usize> {
    let mut totals = vec![0.0; problem.capacities.len()];
    for (group, choice) in problem.groups.iter().zip(&choices) {
        for (total, usage) in totals.iter_mut().zip(&group[*choice].resources) {
            *total += usage;
        }
    }
    let violation = |values: &[f64]| {
        values
            .iter()
            .zip(&problem.capacities)
            .map(|(usage, capacity)| (usage - capacity).max(0.0) / capacity.max(1e-12))
            .sum::<f64>()
    };
    for _ in 0..problem.groups.len() * 2 {
        let current = violation(&totals);
        if current <= 1e-10 {
            break;
        }
        let mut best = None;
        let mut best_violation = current;
        let mut best_quality = f64::NEG_INFINITY;
        for (group_index, group) in problem.groups.iter().enumerate() {
            if fixed[group_index].is_some() {
                continue;
            }
            let existing = &group[choices[group_index]];
            for (choice_index, choice) in group.iter().enumerate() {
                if choice_index == choices[group_index] {
                    continue;
                }
                let next: Vec<_> = totals
                    .iter()
                    .zip(&existing.resources)
                    .zip(&choice.resources)
                    .map(|((total, old), new)| total - old + new)
                    .collect();
                let next_violation = violation(&next);
                let quality = choice.quality - existing.quality;
                if next_violation < best_violation - 1e-10
                    || (next_violation < current - 1e-10
                        && (next_violation - best_violation).abs() <= 1e-10
                        && quality > best_quality)
                {
                    best = Some((group_index, choice_index, next));
                    best_violation = next_violation;
                    best_quality = quality;
                }
            }
        }
        let Some((group, choice, next)) = best else {
            break;
        };
        choices[group] = choice;
        totals = next;
    }
    choices
}

pub fn solve<T>(
    problem: &Problem,
    node_limit: usize,
    mut evaluate: impl FnMut(&[usize]) -> Option<T>,
) -> Outcome<T> {
    if problem.groups.iter().any(Vec::is_empty) {
        return Outcome {
            solution: None,
            bound: None,
            proven: true,
            nodes: 0,
            limitation: None,
        };
    }
    let initial_bound = problem
        .groups
        .iter()
        .map(|group| {
            group
                .iter()
                .map(|choice| choice.quality)
                .fold(0.0, f64::max)
        })
        .sum();
    let mut pending = vec![Node {
        fixed: vec![None; problem.groups.len()],
        bound: initial_bound,
    }];
    let mut incumbent: Option<Solution<T>> = None;
    for criterion in 0..3 {
        let choices = problem
            .groups
            .iter()
            .map(|group| {
                group
                    .iter()
                    .enumerate()
                    .min_by(|(left_index, left), (right_index, right)| {
                        let score = |choice: &Choice| match criterion {
                            0 => resource_score(choice, &problem.capacities),
                            1 => choice.nominal_cost,
                            _ => choice.nominal_time,
                        };
                        score(left)
                            .total_cmp(&score(right))
                            .then_with(|| left_index.cmp(right_index))
                    })
                    .map(|(index, _)| index)
                    .unwrap_or(0)
            })
            .collect();
        let choices = repair(problem, choices, &vec![None; problem.groups.len()]);
        if let Some(solution) = candidate(problem, choices, &mut evaluate)
            && incumbent.as_ref().is_none_or(|best| {
                better(
                    solution.quality,
                    solution.nominal_time,
                    solution.nominal_cost,
                    best,
                )
            })
        {
            incumbent = Some(solution);
        }
    }
    improve(problem, &mut incumbent, &mut evaluate);
    let mut cuts = Vec::new();
    let mut nodes = 0;
    let mut numerical_limit = None;
    let mut limitation = None;
    while nodes < node_limit {
        let Some(node) = pending.pop() else { break };
        if incumbent
            .as_ref()
            .is_some_and(|best| node.bound < best.quality - 1e-8)
        {
            continue;
        }
        nodes += 1;
        let relaxation = match relax(problem, &node.fixed, &cuts) {
            Ok(Some(relaxation)) => relaxation,
            Ok(None) => continue,
            Err(reason) => {
                numerical_limit = Some(node.bound);
                limitation = Some(reason);
                break;
            }
        };
        if incumbent.as_ref().is_some_and(|best| {
            !better(
                relaxation.quality,
                relaxation.nominal_time,
                relaxation.nominal_cost,
                best,
            )
        }) {
            continue;
        }
        let choices: Vec<_> = relaxation
            .fractions
            .iter()
            .map(|values| {
                values
                    .iter()
                    .enumerate()
                    .max_by(|(left_index, left), (right_index, right)| {
                        left.total_cmp(right)
                            .then_with(|| right_index.cmp(left_index))
                    })
                    .map(|(index, _)| index)
                    .unwrap_or(0)
            })
            .collect();
        let fractional = relaxation
            .fractions
            .iter()
            .enumerate()
            .filter(|(group, values)| {
                node.fixed[*group].is_none() && values[choices[*group]] < 1.0 - 1e-7
            })
            .min_by(|(left, values_left), (right, values_right)| {
                values_left[choices[*left]].total_cmp(&values_right[choices[*right]])
            })
            .map(|(group, _)| group);
        if fractional.is_some() && (nodes == 1 || nodes % 16 == 0) {
            let repaired = repair(problem, choices.clone(), &node.fixed);
            if let Some(solution) = candidate(problem, repaired, &mut evaluate)
                && incumbent.as_ref().is_none_or(|best| {
                    better(
                        solution.quality,
                        solution.nominal_time,
                        solution.nominal_cost,
                        best,
                    )
                })
            {
                incumbent = Some(solution);
            }
        }
        if let Some(solution) = candidate(problem, choices.clone(), &mut evaluate) {
            if incumbent.as_ref().is_none_or(|best| {
                better(
                    solution.quality,
                    solution.nominal_time,
                    solution.nominal_cost,
                    best,
                )
            }) {
                incumbent = Some(solution);
            }
            if fractional.is_none() {
                continue;
            }
        } else if fractional.is_none() {
            cuts.push(choices);
            pending.push(Node {
                fixed: node.fixed,
                bound: relaxation.quality,
            });
            continue;
        }
        let Some(group) = fractional else { continue };
        let mut candidates: Vec<_> = (0..problem.groups[group].len()).collect();
        candidates.sort_by(|left, right| {
            relaxation.fractions[group][*right]
                .total_cmp(&relaxation.fractions[group][*left])
                .then_with(|| {
                    problem.groups[group][*right]
                        .quality
                        .total_cmp(&problem.groups[group][*left].quality)
                })
                .then_with(|| left.cmp(right))
        });
        for choice in candidates.into_iter().rev() {
            let mut fixed = node.fixed.clone();
            fixed[group] = Some(choice);
            pending.push(Node {
                fixed,
                bound: relaxation.quality,
            });
        }
    }
    improve(problem, &mut incumbent, &mut evaluate);
    let proven = pending.is_empty() && numerical_limit.is_none();
    let bound = pending
        .iter()
        .map(|node| node.bound)
        .chain(numerical_limit)
        .chain(incumbent.as_ref().map(|solution| solution.quality))
        .reduce(f64::max);
    Outcome {
        solution: incumbent,
        bound,
        proven,
        nodes,
        limitation,
    }
}
