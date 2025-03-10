use crate::runtime::task::TaskId;
use crate::scheduler::data::random::RandomDataSource;
use crate::scheduler::data::DataSource;
use crate::scheduler::{Schedule, Scheduler};

/// A round robin scheduler that chooses the next available runnable task at each context switch.
#[derive(Debug)]
pub struct RoundRobinScheduler {
    iterations: usize,
    data_source: RandomDataSource,
}

impl RoundRobinScheduler {
    /// Construct a new `RoundRobinScheduler` that will execute the test only once, scheduling its
    /// tasks in a round-robin fashion.
    pub fn new() -> Self {
        Self {
            iterations: 0,
            data_source: RandomDataSource::initialize(0),
        }
    }
}

impl Scheduler for RoundRobinScheduler {
    fn new_execution(&mut self) -> Option<Schedule> {
        if self.iterations == 0 {
            self.iterations += 1;
            Some(Schedule::new(self.data_source.reinitialize()))
        } else {
            None
        }
    }

    fn next_task(&mut self, runnable: &[TaskId], current: Option<TaskId>, _is_yielding: bool) -> Option<TaskId> {

        tracing::info!("running {:?} of {:?}", current, runnable);

        if current.is_none() {
            return Some(*runnable.first().unwrap());
        }
        let current = current.unwrap();

        Some(
            *runnable
                .iter()
                .find(|t| **t > current)
                .unwrap_or_else(|| runnable.first().unwrap()),
        )
    }

    fn next_u64(&mut self) -> u64 {
        self.data_source.next_u64()
    }
}

impl Default for RoundRobinScheduler {
    fn default() -> Self {
        Self::new()
    }
}
