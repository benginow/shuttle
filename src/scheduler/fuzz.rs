use crate::runtime::task::TaskId;
use crate::scheduler::data::random::RandomDataSource;
use crate::scheduler::data::DataSource;
use crate::scheduler::{Schedule, ScheduleStep, Scheduler};

#[derive(Debug)]
pub struct FuzzScheduler {
    // any number of iterations..?
    // this doesn't need to be necessary anymore
    schedule: Option<Schedule>,
    // is the current schedule complete -- if no, we cannot reset the schedule yet
    complete: bool,
    // number of steps we have taken so far
    steps: usize,
    // i lowkey don't know what this is for
    data_source: RandomDataSource,
}

impl FuzzScheduler {
    pub fn new() -> Self {
        Self {
            schedule: Some(Schedule::new(0)),
            // this is just a placeholder, so you should be free to change it
            complete: true,
            steps: 0,
            data_source: RandomDataSource::initialize(0),
        }
    }
}

// TODO: double check that you can only run one schedule at a time
impl Scheduler for FuzzScheduler {
    fn new_execution(&mut self) -> Option<Schedule> {
        // lol idk just don't call this smh
        eprintln!("incorrect usage of fuzz scheduler");
        None
    }
    fn next_task(
        &mut self,
        runnable_tasks: &[TaskId],
        //make sure these are/are not needed?
        _current_task: Option<TaskId>,
        _is_yielding: bool,
    ) -> Option<TaskId> {
        tracing::info!(?runnable_tasks, ?self.schedule, ?self.steps, "next task");

        match &self.schedule {
            Some(schedule) => {
                if schedule.steps.len() <= self.steps {
                    if runnable_tasks.len() > 0 {
                        return Some(runnable_tasks[0]);
                    } else {
                        return None;
                    }
                }
                match schedule.steps[self.steps] {
                    ScheduleStep::Random => {
                        self.steps += 1;
                        return Some(runnable_tasks[0]);
                    }
                    ScheduleStep::Task(next) => {
                        // fuzzer probably generates random u64s for task IDs, which will never match.
                        // treat them instead as indexes into runnable_tasks.
                        let next: usize = next.into();
                        let next = next % runnable_tasks.len();
                        let next = runnable_tasks[next];
                        self.steps += 1;
                        if self.steps >= schedule.steps.len() {
                            // we have completed the thing
                            self.complete = true;
                        }
                        Some(next)
                    }
                }
            }
            None => {
                eprintln!("incorrect use of fuzz scheduler");
                None
            }
        }
    }

    fn next_u64(&mut self) -> u64 {
        self.data_source.next_u64()
    }

    fn new_execution_fuzz(&mut self, schedule: Option<Schedule>) -> Option<Schedule> {
        tracing::info!(?schedule, "new execution");
        self.schedule = schedule;
        self.complete = false;
        self.schedule.clone()
    }
}
