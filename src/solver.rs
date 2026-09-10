use std::cmp::{Ordering, Reverse};
use std::collections::BinaryHeap;

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
    pub limitation: Option<&'static str>,
}

pub struct SolverState<T> {
    pending: BinaryHeap<QueuedNode>,
    cuts: Vec<Vec<usize>>,
    incumbent: Option<Solution<T>>,
    nodes: usize,
    numerical_limit: Option<f64>,
    limitation: Option<&'static str>,
    next_order: usize,
    neighborhood_remaining: usize,
}

impl<T> SolverState<T> {
    pub fn has_solution(&self) -> bool {
        self.incumbent.is_some()
    }

    pub fn proven(&self) -> bool {
        self.pending.is_empty() && self.numerical_limit.is_none()
    }

    pub fn nodes(&self) -> usize {
        self.nodes
    }

    pub fn limitation(&self) -> Option<&'static str> {
        self.limitation
    }

    pub fn can_advance(&self) -> bool {
        !self.pending.is_empty()
    }
}

struct Node {
    fixed: Vec<Option<usize>>,
    bound: f64,
}

struct QueuedNode {
    node: Node,
    depth: usize,
    order: usize,
}

impl PartialEq for QueuedNode {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other).is_eq()
    }
}

impl Eq for QueuedNode {}

impl PartialOrd for QueuedNode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for QueuedNode {
    fn cmp(&self, other: &Self) -> Ordering {
        self.node
            .bound
            .total_cmp(&other.node.bound)
            .then_with(|| self.depth.cmp(&other.depth))
            .then_with(|| other.order.cmp(&self.order))
    }
}

#[derive(Clone, Copy)]
struct CoupledMove {
    first_group: usize,
    first_choice: usize,
    second_group: usize,
    second_choice: usize,
    quality_delta: f64,
    time_delta: f64,
    cost_delta: f64,
}

impl PartialEq for CoupledMove {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other).is_eq()
    }
}

impl Eq for CoupledMove {}

impl PartialOrd for CoupledMove {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for CoupledMove {
    fn cmp(&self, other: &Self) -> Ordering {
        self.quality_delta
            .total_cmp(&other.quality_delta)
            .then_with(|| other.time_delta.total_cmp(&self.time_delta))
            .then_with(|| other.cost_delta.total_cmp(&self.cost_delta))
            .then_with(|| other.first_group.cmp(&self.first_group))
            .then_with(|| other.first_choice.cmp(&self.first_choice))
            .then_with(|| other.second_group.cmp(&self.second_group))
            .then_with(|| other.second_choice.cmp(&self.second_choice))
    }
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

fn improve_coupled<T>(
    problem: &Problem,
    incumbent: &mut Option<Solution<T>>,
    evaluation_limit: usize,
    evaluate: &mut impl FnMut(&[usize]) -> Option<T>,
) {
    let mut remaining = evaluation_limit;
    while remaining > 0 {
        let Some(current) = incumbent.as_ref() else {
            return;
        };
        let choices = current.choices.clone();
        let mut baseline_resources = vec![0.0; problem.capacities.len()];
        for (group, choice) in problem.groups.iter().zip(&choices) {
            for (total, usage) in baseline_resources.iter_mut().zip(&group[*choice].resources) {
                *total += usage;
            }
        }
        let mut moves = BinaryHeap::new();
        for first_group in 0..problem.groups.len() {
            let first_existing = &problem.groups[first_group][choices[first_group]];
            for second_group in first_group + 1..problem.groups.len() {
                let second_existing = &problem.groups[second_group][choices[second_group]];
                for (first_choice, first) in problem.groups[first_group].iter().enumerate() {
                    if first_choice == choices[first_group] {
                        continue;
                    }
                    for (second_choice, second) in problem.groups[second_group].iter().enumerate() {
                        if second_choice == choices[second_group] {
                            continue;
                        }
                        let quality_delta = first.quality + second.quality
                            - first_existing.quality
                            - second_existing.quality;
                        let time_delta = first.nominal_time + second.nominal_time
                            - first_existing.nominal_time
                            - second_existing.nominal_time;
                        let cost_delta = first.nominal_cost + second.nominal_cost
                            - first_existing.nominal_cost
                            - second_existing.nominal_cost;
                        if quality_delta < -1e-8
                            || (quality_delta.abs() <= 1e-8
                                && (time_delta > 1e-8
                                    || (time_delta.abs() <= 1e-8 && cost_delta >= -1e-8)))
                        {
                            continue;
                        }
                        if baseline_resources
                            .iter()
                            .zip(&first_existing.resources)
                            .zip(&second_existing.resources)
                            .zip(&first.resources)
                            .zip(&second.resources)
                            .zip(&problem.capacities)
                            .any(
                                |(
                                    ((((total, first_old), second_old), first_new), second_new),
                                    cap,
                                )| {
                                    total - first_old - second_old + first_new + second_new
                                        > cap + 1e-8
                                },
                            )
                        {
                            continue;
                        }
                        let candidate = CoupledMove {
                            first_group,
                            first_choice,
                            second_group,
                            second_choice,
                            quality_delta,
                            time_delta,
                            cost_delta,
                        };
                        if moves.len() < remaining {
                            moves.push(Reverse(candidate));
                        } else if moves.peek().is_some_and(|worst| candidate > worst.0) {
                            moves.pop();
                            moves.push(Reverse(candidate));
                        }
                    }
                }
            }
        }
        let mut ranked: Vec<_> = moves.into_iter().map(|entry| entry.0).collect();
        ranked.sort_by(|left, right| right.cmp(left));
        let mut improved = false;
        for movement in ranked {
            let mut next = choices.clone();
            next[movement.first_group] = movement.first_choice;
            next[movement.second_group] = movement.second_choice;
            remaining -= 1;
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
            }) {
                *incumbent = Some(solution);
                improved = true;
                break;
            }
        }
        if !improved {
            return;
        }
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

pub fn begin<T>(
    problem: &Problem,
    neighborhood_limit: usize,
    mut evaluate: impl FnMut(&[usize]) -> Option<T>,
) -> SolverState<T> {
    if problem.groups.iter().any(Vec::is_empty) {
        return SolverState {
            pending: BinaryHeap::new(),
            cuts: Vec::new(),
            incumbent: None,
            nodes: 0,
            numerical_limit: None,
            limitation: None,
            next_order: 0,
            neighborhood_remaining: neighborhood_limit,
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
    let pending = BinaryHeap::from([QueuedNode {
        node: Node {
            fixed: vec![None; problem.groups.len()],
            bound: initial_bound,
        },
        depth: 0,
        order: 0,
    }]);
    let mut incumbent = None;
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
    SolverState {
        pending,
        cuts: Vec::new(),
        incumbent,
        nodes: 0,
        numerical_limit: None,
        limitation: None,
        next_order: 1,
        neighborhood_remaining: neighborhood_limit,
    }
}

pub fn advance<T>(
    problem: &Problem,
    state: &mut SolverState<T>,
    additional_nodes: usize,
    stop_on_feasible: bool,
    mut evaluate: impl FnMut(&[usize]) -> Option<T>,
) {
    if !stop_on_feasible && state.neighborhood_remaining > 0 {
        improve(problem, &mut state.incumbent, &mut evaluate);
        improve_coupled(
            problem,
            &mut state.incumbent,
            state.neighborhood_remaining,
            &mut evaluate,
        );
        state.neighborhood_remaining = 0;
    }
    let target = state.nodes.saturating_add(additional_nodes);
    while state.nodes < target {
        if stop_on_feasible && state.incumbent.is_some() {
            break;
        }
        let Some(queued) = state.pending.pop() else {
            break;
        };
        let node = queued.node;
        if state
            .incumbent
            .as_ref()
            .is_some_and(|best| node.bound < best.quality - 1e-8)
        {
            continue;
        }
        state.nodes += 1;
        let relaxation = match relax(problem, &node.fixed, &state.cuts) {
            Ok(Some(relaxation)) => relaxation,
            Ok(None) => continue,
            Err(reason) => {
                state.numerical_limit = Some(
                    state
                        .numerical_limit
                        .map_or(node.bound, |bound| bound.max(node.bound)),
                );
                state.limitation = Some(reason);
                break;
            }
        };
        if state.incumbent.as_ref().is_some_and(|best| {
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
        if fractional.is_some() && (state.nodes == 1 || state.nodes.is_multiple_of(16)) {
            let repaired = repair(problem, choices.clone(), &node.fixed);
            if let Some(solution) = candidate(problem, repaired, &mut evaluate)
                && state.incumbent.as_ref().is_none_or(|best| {
                    better(
                        solution.quality,
                        solution.nominal_time,
                        solution.nominal_cost,
                        best,
                    )
                })
            {
                state.incumbent = Some(solution);
            }
        }
        if let Some(solution) = candidate(problem, choices.clone(), &mut evaluate) {
            if state.incumbent.as_ref().is_none_or(|best| {
                better(
                    solution.quality,
                    solution.nominal_time,
                    solution.nominal_cost,
                    best,
                )
            }) {
                state.incumbent = Some(solution);
            }
            if fractional.is_none() {
                continue;
            }
        } else if fractional.is_none() {
            state.cuts.push(choices);
            state.pending.push(QueuedNode {
                node: Node {
                    fixed: node.fixed,
                    bound: relaxation.quality,
                },
                depth: queued.depth,
                order: state.next_order,
            });
            state.next_order += 1;
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
        for choice in candidates {
            let mut fixed = node.fixed.clone();
            fixed[group] = Some(choice);
            state.pending.push(QueuedNode {
                node: Node {
                    fixed,
                    bound: relaxation.quality,
                },
                depth: queued.depth + 1,
                order: state.next_order,
            });
            state.next_order += 1;
        }
    }
}

pub fn finish<T>(state: SolverState<T>) -> Outcome<T> {
    let proven = state.pending.is_empty() && state.numerical_limit.is_none();
    let bound = state
        .pending
        .iter()
        .map(|queued| queued.node.bound)
        .chain(state.numerical_limit)
        .chain(state.incumbent.as_ref().map(|solution| solution.quality))
        .reduce(f64::max);
    Outcome {
        solution: state.incumbent,
        bound,
        proven,
        limitation: state.limitation,
    }
}
