
// use crate::runtime::task::TaskId;
// use crate::scheduler::data::random::RandomDataSource;
// use crate::scheduler::data::DataSource;
// use crate::scheduler::{Schedule, ScheduleStep, Scheduler};
// use rand::Rng;


// const MAX_ITERS: usize = 10000;

// pub struct FuzzScheduler {
//     max_iterations: usize,
//     max_depth: usize,

//     complete: bool,
//     steps: usize,
//     iterations: usize,
//     data_source: RandomDataSource,
// }

// // fuzzable struct -- pct params
// #[derive(Clone, Debug, Default, PartialEq, Eq, Arbitrary)]
// pub struct PctParams {
//     max_depth: usize,
//     // not sure if we want to do this.. 
//     // may want to set this and just do it "enough" times? hmm
//     max_iterations: usize,
// }

// impl FuzzScheduler {
//     pub fn new(md: CompletionMode) -> Self {
//         Self {
//             complete: true,
//             steps: 0,
//             iterations: 0,
//             data_source: RandomDataSource::initialize(0),
//         }
//     }
// }

// impl Scheduler for FuzzScheduler {
//     ///
//     /// new_execution should not get called -- instead new_execution_fuzz gets called
//     /// so that we can pass in a new fuzzed schedule
//     /// 
//     fn new_execution(&mut self) -> Option<Schedule> {
//         eprintln!("incorrect usage of fuzz scheduler");
//         None
//     }

//     /// 
//     /// Runs the next task given by the fuzzed schedule
//     /// If there is no next task in the fuzzed schedule, just 
//     /// 
//     fn next_task(
//         &mut self,
//         runnable_tasks: &[TaskId],
//         //make sure these are/are not needed?
//         _current_task: Option<TaskId>,
//         _is_yielding: bool,
//     ) -> Option<TaskId> {
//         tracing::info!(?runnable_tasks, ?self.schedule, ?self.steps, "next task");

//         if runnable_tasks.len() == 0 {
//             tracing::info!("NO RUNNABLE TASKS {:?}", runnable_tasks);
//         }

//         match &(self.schedule.clone()) {
            
//             Some(schedule) => {
//                 if schedule.steps.len() <= self.steps {
//                     // If we have gone over max iters, we will just abort regardless -- 
//                     // this is to allow forward progress in the fuzzer
//                     if self.iterations > MAX_ITERS {
//                         None
//                     }
//                     else {
//                         self.complete_schedule(runnable_tasks, schedule)
//                     }
//                 }
//                 else { match schedule.steps[self.steps] {
//                     ScheduleStep::Random => {
//                         // this should never execute
//                         tracing::info!("chose a random schedule step -- {:?}", Some(runnable_tasks[0]));
//                         self.steps += 1;
//                         self.iterations += 1;
//                         Some(runnable_tasks[0])
//                     }
//                     ScheduleStep::Task(next) => {
//                         // fuzzer probably generates random u64s for task IDs, which will never match.
//                         // treat them instead as indexes into runnable_tasks.
//                         let next: usize = next.into();
//                         let next = next % runnable_tasks.len();
//                         let next = runnable_tasks[next];
//                         self.steps += 1;
//                         self.iterations += 1;
//                         if self.steps >= schedule.steps.len() {
//                             // we have completed the thing
//                             self.complete = true;
//                         }
//                         Some(next)
//                     }
//                 }
//             } }
//             None => {
//                 tracing::info!("incorrect use of fuzz scheduler -- no schedule available");
//                 None
//             }
//         }
//     }

//     fn next_u64(&mut self) -> u64 {
//         // self.data_source.next_u64()
//     }

//     fn new_execution_fuzz(&mut self, schedule: Option<Schedule>, params: Option<PctParams>) -> Option<Schedule> {
//         // tracing::info!(?schedule, "new execution");
//         match params {
//             Some(p) => {
//                 self.max_iterations = p.max_iterations;
//                 self.max_depth = p.max_depth;
//             }
//         }
//         // self.complete = false;
//  
//     }
// }
