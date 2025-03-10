use crate::runtime::execution::Execution;
use crate::runtime::task::TaskId;
use crate::runtime::thread::continuation::{ContinuationPool, CONTINUATION_POOL};
use crate::scheduler::metrics::MetricsScheduler;
use crate::scheduler::{Schedule, Scheduler};
use crate::Config;
use afl::fuzz;
use std::cell::RefCell;
use std::fmt;
use std::panic::{self, AssertUnwindSafe, RefUnwindSafe, UnwindSafe};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::Instant;
use tracing::{span, Level};

/// A `Runner` is the entry-point for testing concurrent code.
///
/// It takes as input a function to test and a `Scheduler` to run it under. It then executes that
/// function as many times as dictated by the scheduler; each execution has its scheduling decisions
/// resolved by the scheduler, which can make different choices for each execution.
#[derive(Debug)]
pub struct Runner<S: ?Sized + Scheduler> {
    scheduler: Rc<RefCell<MetricsScheduler<S>>>,
    config: Config,
}

impl<S: Scheduler + 'static> Runner<S> {
    /// Construct a new `Runner` that will use the given `Scheduler` to control the test.
    pub fn new(scheduler: S, config: Config) -> Self {
        let metrics_scheduler = MetricsScheduler::new(scheduler);

        Self {
            scheduler: Rc::new(RefCell::new(metrics_scheduler)),
            config,
        }
    }

    pub fn new_fuzz(scheduler: S, config: Config) -> Self {
        let metrics_scheduler = MetricsScheduler::new(scheduler);

        Self {
            scheduler: Rc::new(RefCell::new(metrics_scheduler)),
            config,
        }
    }

    pub fn run_fuzz<F>(self, f: F) -> usize
    where
        F: Fn() + Send + Sync + RefUnwindSafe + UnwindSafe + 'static,
    {
        let this = AssertUnwindSafe(self);
        let mut i = 0;
        let f = Arc::new(f);

        tracing::info!("starting fuzzer");

        fuzz!(|s: Schedule| {
            this.fuzz_inner(f.clone(), s, &mut i);
        });

        i
    }

    /// Target of the fuzz! macro. Can also use this to replay crash schedules through the fuzz
    /// scheduler (which computes Schedules a little differently).
    pub fn fuzz_inner<F>(&self, f: Arc<F>, s: Schedule, i: &mut usize)
    where
        F: Fn() + Send + Sync + RefUnwindSafe + UnwindSafe + 'static,
    {
        tracing::info!("starting schedule {:?}", s);

        CONTINUATION_POOL.set(&ContinuationPool::new(), || {
            let schedule = match self.scheduler.borrow_mut().new_execution_fuzz(Some(s.clone()), None) {
                None => panic!("do something more intelligent here"),
                Some(s) => s,
            };

            let execution = Execution::new(self.scheduler.clone(), schedule);
            let f = Arc::clone(&f);

            span!(Level::INFO, "execution", i).in_scope(|| execution.run(&self.config, move || (*f)()));

            *i += 1;
        });
    }

    pub fn fuzz_pct_inner<F>(&self, f: Arc<F>, p: [usize;16], i: &mut usize) 
    where
        F: Fn() + Send + Sync + RefUnwindSafe + UnwindSafe + 'static,
    {
        tracing::info!("using preemption points {:?}, will be trimmed", p);

        CONTINUATION_POOL.set(&ContinuationPool::new(), || {
            let schedule = match self.scheduler.borrow_mut().new_execution_fuzz(None, Some(p.clone())) {
                None => panic!("do something more intelligent here"),
                Some(s) => s,
            };

            let execution = Execution::new(self.scheduler.clone(), schedule);
            let f = Arc::clone(&f);

            span!(Level::INFO, "execution", i).in_scope(|| execution.run(&self.config, move || (*f)()));

            *i += 1;
        });
    }

    pub fn run_pct_fuzz<F>(&self, f: F) -> usize
    where
        F: Fn() + Send + Sync + RefUnwindSafe + UnwindSafe + 'static,
    {
        let this = AssertUnwindSafe(self);
        let mut i = 0;
        let f = Arc::new(f);

        tracing::info!("starting fuzzer");

        // just generate any number of preemption points. 
        // no need to use all of them.
        // assume user doesn't use more than 16 preemption points
        fuzz!(|preemptions: Vec<usize>| {
            this.fuzz_pct_inner(f.clone(), preemptions, &mut i);
        });

        i
    }

    /// Test the given function and return the number of times the function was invoked during the
    /// test (i.e., the number of iterations run).
    pub fn run<F>(self, f: F) -> usize
    where
        F: Fn() + Send + Sync + 'static,
    {
        // Share continuations across executions to avoid reallocating them
        // TODO it would be a lot nicer if this were a more generic "context" thing that we passed
        // TODO around explicitly rather than being a thread local
        CONTINUATION_POOL.set(&ContinuationPool::new(), || {
            let f = Arc::new(f);

            let start = Instant::now();

            let mut i = 0;
            loop {
                if self.config.max_time.map(|t| start.elapsed() > t).unwrap_or(false) {
                    break;
                }

                let schedule = match self.scheduler.borrow_mut().new_execution() {
                    None => break,
                    Some(s) => s,
                };

                let execution = Execution::new(self.scheduler.clone(), schedule);
                let f = Arc::clone(&f);

                span!(Level::INFO, "execution", i).in_scope(|| execution.run(&self.config, move || f()));

                i += 1;
            }
            i
        })
    }
}

/// A `PortfolioRunner` is the same as a `Runner`, except that it can run multiple different
/// schedulers (a "portfolio" of schedulers) in parallel. If any of the schedulers finds a failing
/// execution of the test, the entire run fails.
pub struct PortfolioRunner {
    schedulers: Vec<Box<dyn Scheduler + Send + 'static>>,
    stop_on_first_failure: bool,
    config: Config,
}

impl PortfolioRunner {
    /// Construct a new `PortfolioRunner` with no schedulers. If `stop_on_first_failure` is true,
    /// all schedulers will be terminated as soon as any fails; if false, they will keep running
    /// and potentially find multiple bugs.
    pub fn new(stop_on_first_failure: bool, config: Config) -> Self {
        Self {
            schedulers: Vec::new(),
            stop_on_first_failure,
            config,
        }
    }

    /// Add the given scheduler to the portfolio of schedulers to run the test with.
    pub fn add(&mut self, scheduler: impl Scheduler + Send + 'static) {
        self.schedulers.push(Box::new(scheduler));
    }

    /// Test the given function against all schedulers in parallel. If any of the schedulers finds
    /// a failing execution, this function panics.
    pub fn run<F>(self, f: F)
    where
        F: Fn() + Send + Sync + 'static,
    {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        enum ThreadResult {
            Passed,
            Failed,
        }

        let (tx, rx) = mpsc::sync_channel::<ThreadResult>(0);
        let stop_signal = Arc::new(AtomicBool::new(false));
        let config = self.config;
        let f = Arc::new(f);

        let threads = self
            .schedulers
            .into_iter()
            .enumerate()
            .map(|(i, scheduler)| {
                let f = Arc::clone(&f);
                let tx = tx.clone();
                let stop_signal = stop_signal.clone();
                let config = config.clone();

                thread::spawn(move || {
                    let scheduler = PortfolioStoppableScheduler { scheduler, stop_signal };

                    let runner = Runner::new(scheduler, config);

                    span!(Level::INFO, "job", i).in_scope(|| {
                        let ret = panic::catch_unwind(panic::AssertUnwindSafe(|| runner.run(move || f())));

                        match ret {
                            Ok(_) => tx.send(ThreadResult::Passed),
                            Err(e) => {
                                tx.send(ThreadResult::Failed).unwrap();
                                panic::resume_unwind(e);
                            }
                        }
                    })
                })
            })
            .collect::<Vec<_>>();

        // Wait for each thread to pass or fail, and if any fails, tell all threads to stop early
        for _ in 0..threads.len() {
            if rx.recv().unwrap() == ThreadResult::Failed && self.stop_on_first_failure {
                stop_signal.store(true, Ordering::SeqCst);
            }
        }

        // Join all threads and propagate the first panic we see (note that this might not be the
        // same panic that caused us to stop, if multiple threads panic around the same time, but
        // that's probably OK).
        let mut panic = None;
        for thread in threads {
            if let Err(e) = thread.join() {
                panic = Some(e);
            }
        }
        assert!(stop_signal.load(Ordering::SeqCst) == panic.is_some());
        if let Some(e) = panic {
            std::panic::resume_unwind(e);
        }
    }
}

impl fmt::Debug for PortfolioRunner {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("PortfolioRunner")
            .field("schedulers", &self.schedulers.len())
            .field("stop_on_first_failure", &self.stop_on_first_failure)
            .field("config", &self.config)
            .finish()
    }
}

/// A wrapper around a `Scheduler` that can be told to stop early by setting a flag. We use this to
/// abort all jobs in a `PortfolioRunner` as soon as any job fails.
#[derive(Debug)]
struct PortfolioStoppableScheduler<S> {
    scheduler: S,
    stop_signal: Arc<AtomicBool>,
}

impl<S: Scheduler> Scheduler for PortfolioStoppableScheduler<S> {
    fn new_execution(&mut self) -> Option<Schedule> {
        if self.stop_signal.load(Ordering::SeqCst) {
            None
        } else {
            self.scheduler.new_execution()
        }
    }

    fn next_task(
        &mut self,
        runnable_tasks: &[TaskId],
        current_task: Option<TaskId>,
        is_yielding: bool,
    ) -> Option<TaskId> {
        if self.stop_signal.load(Ordering::SeqCst) {
            None
        } else {
            self.scheduler.next_task(runnable_tasks, current_task, is_yielding)
        }
    }

    fn next_u64(&mut self) -> u64 {
        self.scheduler.next_u64()
    }
}
