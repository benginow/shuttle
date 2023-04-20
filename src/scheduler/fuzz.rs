use crate::runtime::task::TaskId;
use crate::scheduler::data::random::RandomDataSource;
use crate::scheduler::data::DataSource;
use crate::scheduler::{Schedule, ScheduleStep, Scheduler};
use rand::Rng;

#[derive(Debug)]

enum completion_mode {
    ABORT,
    ROUND_ROBIN,
    RANDOM,
    WRAPAROUND,
}
pub struct FuzzScheduler {
    schedule: Option<Schedule>,
    complete: bool,
    steps: usize,
    data_source: RandomDataSource,
    mode: completion_mode,
    // mostly just used for wraparound mode
}

impl FuzzScheduler {
    pub fn new() -> Self {
        Self {
            schedule: Some(Schedule::new(0)),
            complete: true,
            steps: 0,
            data_source: RandomDataSource::initialize(0),
            mode: completion_mode::RANDOM,
        }
    }
}

impl Scheduler for FuzzScheduler {
    ///
    /// new_execution should not get called -- instead new_execution_fuzz gets called
    /// so that we can pass in a new fuzzed schedule
    /// 
    fn new_execution(&mut self) -> Option<Schedule> {
        eprintln!("incorrect usage of fuzz scheduler");
        None
    }
    /// 
    /// Runs the next task given by the fuzzed schedule
    /// If there is no next task in the fuzzed schedule, just 
    /// 
    fn next_task(
        &mut self,
        runnable_tasks: &[TaskId],
        //make sure these are/are not needed?
        _current_task: Option<TaskId>,
        _is_yielding: bool,
    ) -> Option<TaskId> {
        tracing::info!(?runnable_tasks, ?self.schedule, ?self.steps, "next task");

        if runnable_tasks.len() == 0 {
            tracing::info!("NO RUNNABLE TASKS {:?}", runnable_tasks);
        }

        match &self.schedule {
            
            Some(schedule) => {
                if schedule.steps.len() <= self.steps {
                    match &self.mode {
                        completion_mode::ABORT => {
                            tracing::info!("we have run out of schedule steps, abort");
                            return None;
                        }
                        completion_mode::ROUND_ROBIN => {
                            if runnable_tasks.len() > 0 {
                                tracing::info!("we have run out of schedule steps, and we are now completing the schedule by means of round-robin scheduling");
                                return Some(runnable_tasks[0]);
                            } else {
                                return None;
                            }
                        }
                        completion_mode::RANDOM => {
                            if runnable_tasks.len() > 0 {
                                let mut rng = rand::thread_rng();
                                let task_num = rng.gen_range(0.. runnable_tasks.len());
                                
                                tracing::info!("we have run out of schedule steps, and we are now generating a random schedule");
                                return Some(runnable_tasks[task_num]);
                            } else {
                                return None;
                            }
                        }
                        completion_mode::WRAPAROUND => {
                            if runnable_tasks.len() > 0 {
                                tracing::info!("we have run out of schedule steps and are now wrapping around :)")
                                self.steps = 0;
                                // this is copied and pasted. fix her.
                                match schedule.steps[self.steps] {
                                    ScheduleStep::Random => {
                                        // this should never execute
                                        tracing::info!("chose a random schedule step -- {:?}", Some(runnable_tasks[0]));
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
                            else {
                                return None;
                            }
                        }
                    }
                }
                match schedule.steps[self.steps] {
                    ScheduleStep::Random => {
                        // this should never execute
                        tracing::info!("chose a random schedule step -- {:?}", Some(runnable_tasks[0]));
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
