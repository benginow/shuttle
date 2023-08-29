use crate::runtime::task::{TaskId, DEFAULT_INLINE_TASKS};
use crate::scheduler::data::random::RandomDataSource;
use crate::scheduler::data::DataSource;
use crate::scheduler::{Schedule, Scheduler};
use rand::rngs::OsRng;
use rand::seq::{index::sample, SliceRandom};
use rand::{Rng, RngCore, SeedableRng};
use rand_pcg::Pcg64Mcg;
use std::collections::{HashMap, HashSet};

/// A scheduler that implements the Probabilistic Concurrency Testing (PCT) algorithm.
///
/// The PCT algorithm comes from the paper "A Randomized Scheduler with Probabilistic Guarantees of
/// Finding Bugs", Burckhardt et al, ASPLOS 2010. This implementation follows the one in [Coyote]
/// which differs slightly from the paper (see notes in `next_task`) and supports dynamically
/// determining the bound on the number of steps.
///
/// [Coyote]: https://github.com/microsoft/coyote/blob/master/Source/Core/SystematicTesting/Strategies/Probabilistic/PCTStrategy.cs
#[derive(Debug)]
pub struct FuzzPctScheduler {
    max_iterations: usize,
    max_depth: usize,
    iterations: usize,
    // invariants: every TaskId in [0, len) appears as a key exactly once; all values are distinct
    priorities: HashMap<TaskId, usize>,
    next_priority: usize,
    
    max_steps: usize,
    steps: usize,
    rng: Pcg64Mcg,
    data_source: RandomDataSource,
    change_points: Vec<usize>,
}

impl FuzzPctScheduler {
    /// Construct a new PCTScheduler with a freshly seeded RNG.
    pub fn new(max_depth: usize, max_iterations: usize) -> Self {
        Self::new_from_seed(OsRng.next_u64(), max_depth, max_iterations)
    }

    /// Construct a new PCTScheduler with a given seed.
    pub fn new_from_seed(seed: u64, max_depth: usize, max_iterations: usize) -> Self {
        assert!(max_depth > 0);

        let rng = Pcg64Mcg::seed_from_u64(seed);

        Self {
            max_iterations,
            max_depth,
            iterations: 0,
            priorities: (0..DEFAULT_INLINE_TASKS).map(|i| (TaskId::from(i), i)).collect(),
            next_priority: DEFAULT_INLINE_TASKS,
            change_points: Vec::new(),
            max_steps: 0,
            steps: 0,
            rng,
            data_source: RandomDataSource::initialize(seed),
        }
    }
}

impl Scheduler for FuzzPctScheduler {
    fn new_execution(&mut self) -> Option<Schedule> {
        panic!("incorrect usage of fuzz pct scheduler")
    }

    fn new_execution_fuzz(&mut self, schedule: Option<Schedule>, change_pts: Option<Vec<usize>>) -> Option<Schedule> {
        if self.iterations >= self.max_iterations {
            return None;
        }
        tracing::info!("number of iterations {0}", self.iterations);

        self.steps = 0;

        if self.iterations > 0 {
            assert!(self.max_steps > 0);

            debug_assert_eq!(
                self.priorities.iter().collect::<HashSet<_>>().len(),
                self.priorities.len()
            );

            let mut priorities = (0..self.priorities.len()).collect::<Vec<_>>();
            priorities.shuffle(&mut self.rng);
            for (i, priority) in priorities.into_iter().enumerate() {
                let old = self.priorities.insert(TaskId::from(i), priority);
                debug_assert!(old.is_some(), "priority queue invariant");
            }
            self.next_priority = self.priorities.len();

            let num_points = std::cmp::min(self.max_depth - 1, self.max_steps - 1);

            // here, instead, change points are pre-generated by the fuzzer
            let modulus = self.max_steps-1;
            self.change_points.iter_mut().for_each(|num| *num = (*num%modulus) + 1);
            self.change_points.sort();
        }

        self.iterations += 1;

        Some(Schedule::new(self.data_source.reinitialize()))
    }

    fn next_task(&mut self, runnable: &[TaskId], current: Option<TaskId>, is_yielding: bool) -> Option<TaskId> {
        // If any new tasks were created, assign them priorities by randomly swapping them with an
        // existing task's priority, so we maintain the invariant that every priority is distinct
        let max_known_task = self.priorities.len();
        let max_new_task = usize::from(*runnable.iter().max().unwrap());
        for new_task_id in max_known_task..1 + max_new_task {
            let new_task_id = TaskId::from(new_task_id);
            // Make sure there's a chance to give the new task the lowest priority
            let target_task_id = TaskId::from(self.rng.gen_range(0..self.priorities.len()) + 1);
            let new_task_priority = if target_task_id == new_task_id {
                self.next_priority
            } else {
                self.priorities
                    .insert(target_task_id, self.next_priority)
                    .expect("priority queue invariant")
            };
            let old = self.priorities.insert(new_task_id, new_task_priority);
            debug_assert!(old.is_none(), "priority queue invariant");
            self.next_priority += 1;
        }

        if runnable.len() > 1 {
            if self.change_points.contains(&self.steps) || is_yielding {
                // Deprioritize the current task by lowering its priority to self.next_priority
                let current = current.expect("self.steps > 0 should mean a task has run");
                let old = self.priorities.insert(current, self.next_priority);
                debug_assert!(old.is_some(), "priority queue invariant");
                self.next_priority += 1;
            }

            self.steps += 1;
            if self.steps > self.max_steps {
                self.max_steps = self.steps;
            }
        }

        Some(
            *runnable
                .iter()
                .min_by_key(|tid| self.priorities.get(tid))
                .expect("priority queue invariant"),
        )
    }

    fn next_u64(&mut self) -> u64 {
        self.data_source.next_u64()
    }
}
