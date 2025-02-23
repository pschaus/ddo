use std::{time::{Duration, Instant}, hash::Hash, collections::HashMap};

use ::ddo::{Problem, Cutoff, TimeBudget, NoCutoff, Fringe, NoDupFringe, StateRanking, MaxUB, SimpleFringe, WidthHeuristic, FixedWidth, NbUnassignedWitdh, Variable, Decision, Relaxation, Solver, Completion, SeqNoBarrierSolverLel, SeqBarrierSolverLel, SeqBarrierSolverFc, SeqNoBarrierSolverFc};

use pyo3::{prelude::*, types::{PyBool}};

/// This module exposes binding to the ddo (rust) engine to perform
/// fast discrete optimization using decision diagrams.
#[pymodule]
fn ddo(_py: Python<'_>, m: &PyModule) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(maximize, m)?)?;
    Ok(())
}

#[pyclass]
/// This is the object which is returned after you made a call to 
/// maximize. It does give you various information which you might 
/// find useful. 
pub struct Solution {
    #[pyo3(get)]
    /// Was the search for an optimal solution aborted because of an external cutoff ?
    pub aborted: bool,
    #[pyo3(get)]
    /// What is the gap to optimality
    pub gap: f32,
    #[pyo3(get)]
    /// The time it took to optimize the function (in seconds). 
    pub duration: f64,
    #[pyo3(get)]
    /// What is the objective value of the function you tried to maximize ? 
    /// -> If no solution was found, then the objective value will be None
    pub objective: Option<isize>,
    #[pyo3(get)]
    /// The best known upper bound on the objective value
    pub upper_bound: isize,
    #[pyo3(get)]
    /// The best known lower bound on the objective value
    pub lower_bound: isize,
    #[pyo3(get)]
    /// What are the assigments leading to the best solution ? 
    /// `assignment[x] = y` means value `y` was assigned to variable `x`.
    /// -> If no solution was found, then the assignment value will be None
    pub assignment: Option<Vec<isize>>,
}

#[pyfunction]
#[allow(clippy::too_many_arguments)]
fn maximize(
    pb         : PyObject, 
    relax      : PyObject,
    ranking    : PyObject,
    lel        : bool,
    use_barrier: bool,
    dedup      : bool,
    width      : Option<usize>,
    timeout    : Option<u64>,
) -> Solution {
    Python::with_gil(|gil| {
        let problem = PyProblem {gil, obj: pb};
        let relax = PyRelax {gil, obj: relax};
        let ranking = PyRanking {gil, obj: ranking};
        let max_width = max_width(problem.nb_variables(), width);
        let cutoff = cutoff(timeout);
        let mut fringe = fringe(dedup, &ranking);

        let mut solver = solver(
            &problem, 
            &relax, 
            &ranking, 
            max_width.as_ref(), 
            cutoff.as_ref(), 
            fringe.as_mut(),
            lel,
            use_barrier
        );

        let start = Instant::now();
        let Completion{is_exact, best_value} = solver.maximize();
        
        let duration = start.elapsed().as_secs_f64();
        let gap = solver.gap();
        let assignment = solver.best_solution().map(|mut decisions| {
            decisions.sort_unstable_by_key(|d| d.variable.id());
            decisions.iter().map(|d| d.value).collect()
        });
        
        Solution {
            aborted:     !is_exact,
            objective:   best_value,
            upper_bound: solver.best_upper_bound(),
            lower_bound: solver.best_lower_bound(),
            assignment,
            gap,
            duration
        }
    })
}

#[allow(clippy::too_many_arguments)]
fn solver<'a, 'b>(
    problem    : &'a PyProblem<'b>, 
    relaxation : &'a PyRelax<'b>, 
    ranking    : &'a PyRanking<'b>, 
    width_heu  : &'a dyn WidthHeuristic<PyState<'b>>, 
    cutoff     : &'a dyn Cutoff, 
    fringe     : &'a mut dyn Fringe<State = PyState<'b>>, 
    lel        : bool,
    use_barrier: bool,
) -> Box<dyn Solver + 'a> {
    match (lel, use_barrier) {
        (true, true) => 
            Box::new(SeqBarrierSolverLel::custom(problem, relaxation, ranking, width_heu, cutoff, fringe)),
        (true, false) => 
            Box::new(SeqNoBarrierSolverLel::custom(problem, relaxation, ranking, width_heu, cutoff, fringe)),
        (false, true) => 
            Box::new(SeqBarrierSolverFc::custom(problem, relaxation, ranking, width_heu, cutoff, fringe)),
        (false, false) => 
            Box::new(SeqNoBarrierSolverFc::custom(problem, relaxation, ranking, width_heu, cutoff, fringe)),
    }
}

fn cutoff(timeout: Option<u64>) -> Box<dyn Cutoff> {
    if let Some(timeout) = timeout {
        Box::new(TimeBudget::new(Duration::from_secs(timeout)))
    } else {
        Box::new(NoCutoff)
    }
}

fn fringe<'a>(dedup: bool, ranking: &'a PyRanking<'a>) -> Box<dyn Fringe<State = PyState<'a>> + 'a> {
    if dedup {
        Box::new(NoDupFringe::new(MaxUB::new(ranking)))
    } else {
        Box::new(SimpleFringe::new(MaxUB::new(ranking)))
    }
}

fn max_width<'a>(n: usize, w: Option<usize>) -> Box<dyn WidthHeuristic<PyState<'a>>> {
    if let Some(w) = w {
        Box::new(FixedWidth(w))
    } else {
        Box::new(NbUnassignedWitdh(n))
    }
}

#[derive(Clone)]
pub struct PyState<'a> {
    gil: Python<'a>,
    obj: PyObject
}
unsafe impl Send for PyState<'_> {}
impl Eq for PyState<'_> {}
impl PartialEq for PyState<'_> {
    fn eq(&self, other: &Self) -> bool {
        let res = self.obj.call_method(self.gil, "__eq__", (&other.obj,), None)
            .unwrap();
        let res = res.cast_as::<PyBool>(self.gil)
            .unwrap();
        res.is_true()
    }
}
impl Hash for PyState<'_> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        let res = self.obj.call_method(self.gil, "__hash__", (), None)
            .unwrap();
        let res = res.extract::<isize>(self.gil)
            .unwrap();
        state.write_isize(res)
    }
}

pub struct PyProblem<'a> {
    gil: Python<'a>,
    obj: PyObject
}
unsafe impl Send for PyProblem<'_> {}
impl <'a> Problem for PyProblem<'a> {
    type State = PyState<'a>;

    fn nb_variables(&self) -> usize {
        let res = self.obj.call_method(self.gil, "nb_variables", (), None)
            .unwrap();
        res.extract::<usize>(self.gil)
            .unwrap()
    }

    fn initial_state(&self) -> Self::State {
        let res = {
            self.obj.call_method(self.gil, "initial_state", (), None)
            .unwrap()
        };
        PyState { gil: self.gil, obj: res }
    }

    fn initial_value(&self) -> isize {
        let res = self.obj.call_method(self.gil, "initial_value", (), None)
            .unwrap();
        res.extract::<isize>(self.gil)
            .unwrap()
    }

    fn transition(&self, state: &Self::State, decision: ::ddo::Decision) -> Self::State {
        let res = {
            self.obj.call_method(self.gil, "transition", (&state.obj, decision.variable.0, decision.value), None)
            .unwrap()
        };
        PyState { gil: self.gil, obj: res }
    }

    fn transition_cost(&self, state: &Self::State, decision: ::ddo::Decision) -> isize {
        let res = self.obj.call_method(self.gil, "transition_cost", (&state.obj, decision.variable.0, decision.value), None)
            .unwrap();
        res.extract::<isize>(self.gil)
            .unwrap()
    }

    fn next_variable(&self, depth: usize, next_layer: &mut dyn Iterator<Item = &Self::State>)
        -> Option<::ddo::Variable> {
        let next_layer = next_layer.map(|x| &x.obj).collect::<Vec<_>>();
        
        let res = self.obj.call_method(self.gil, "next_variable", (depth, next_layer,), None)
            .unwrap();
        if res.is_none(self.gil) {
            None
        } else {
            let var_id = res.extract::<usize>(self.gil)
            .unwrap();
            Some(Variable(var_id))
        }
    }

    fn for_each_in_domain(&self, var: ::ddo::Variable, state: &Self::State, f: &mut dyn ::ddo::DecisionCallback) {
        let dom = {
            let res = self.obj.call_method(self.gil, "domain", (var.0, &state.obj), None)
                .unwrap();
            res.extract::<Vec<isize>>(self.gil).unwrap()
        };
        for val in dom {
            f.apply(Decision{variable: var, value: val})
        }
    }
    
}


pub struct PyRelax<'a> {
    gil: Python<'a>,
    obj: PyObject,
}
unsafe impl Send for PyRelax<'_> {}
impl <'a> Relaxation for PyRelax<'a> {
    type State = PyState<'a>;

    fn merge(&self, states: &mut dyn Iterator<Item = &Self::State>) -> Self::State {
        let states = states.map(|x| &x.obj).collect::<Vec<_>>();
        let res = {
            self.obj.call_method(self.gil, "merge", (states,), None)
            .unwrap()
        };
        PyState { gil: self.gil, obj: res }
    }

    fn relax(
        &self,
        source: &Self::State,
        dest: &Self::State,
        new: &Self::State,
        decision: Decision,
        cost: isize,
    ) -> isize {
        let var = decision.variable.0.into_py(self.gil);
        let val = decision.value.into_py(self.gil);
        let cost = cost.into_py(self.gil);

        let mut dict = HashMap::<&str, &PyObject>::default();
        dict.insert("source", &source.obj);
        dict.insert("dest", &dest.obj);
        dict.insert("new", &new.obj);
        dict.insert("variable", &var);
        dict.insert("value", &val);
        dict.insert("cost", &cost);

        let res = self.obj.call_method(self.gil, "relax", (dict,), None)
            .unwrap();
        res.extract(self.gil).unwrap()
    }

    fn fast_upper_bound(&self, state: &Self::State) -> isize {
        let res = self.obj.call_method(self.gil, "fast_upper_bound", (&state.obj,), None);
        if let Ok(res) = res {
            res.extract(self.gil).unwrap()
        } else {
            isize::MAX
        }
    }
}

pub struct PyRanking<'a> {
    gil: Python<'a>,
    obj: PyObject
}
unsafe impl Send for PyRanking<'_> {}
impl <'a> StateRanking for PyRanking<'a> {
    type State = PyState<'a>;

    fn compare(&self, a: &Self::State, b: &Self::State) -> std::cmp::Ordering {
        let res = self.obj.call_method(self.gil, "compare", (&a.obj, &b.obj), None)
            .unwrap();
        let res = res.extract::<isize>(self.gil)
            .unwrap();
        
        match res {
        _ if res == 0 => std::cmp::Ordering::Equal,
        _ if res <  0 => std::cmp::Ordering::Less,
        _ =>             std::cmp::Ordering::Greater
        }
    }
}