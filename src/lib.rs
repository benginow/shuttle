// #![deny(warnings, missing_debug_implementations, missing_docs)]
// #![deny(warnings, missing_debug_implementations)]

//! Shuttle is a library for testing concurrent Rust code, heavily inspired by [Loom][].
//!
//! Shuttle focuses on randomized testing, rather than the exhaustive testing that Loom offers. This
//! is a soundness—scalability trade-off: Shuttle is not sound (a passing Shuttle test does not
//! prove the code is correct), but it scales to much larger test cases than Loom. Empirically,
//! randomized testing is successful at finding most concurrency bugs, which tend not to be
//! adversarial.
//!
//! ## Testing concurrent code
//!
//! Consider this simple piece of concurrent code:
//!
//! ```no_run
//! use std::sync::{Arc, Mutex};
//! use std::thread;
//!
//! let lock = Arc::new(Mutex::new(0u64));
//! let lock2 = lock.clone();
//!
//! thread::spawn(move || {
//!     *lock.lock().unwrap() = 1;
//! });
//!
//! assert_eq!(0, *lock2.lock().unwrap());
//! ```
//!
//! There is an obvious race condition here: if the spawned thread runs before the assertion, the
//! assertion will fail. But writing a unit test that finds this execution is tricky. We could run
//! the test many times and try to "get lucky" by finding a failing execution, but that's not a very
//! reliable testing approach. Even if the test does fail, it will be difficult to debug: we won't
//! be able to easily catch the failure in a debugger, and every time we make a change, we will need
//! to run the test many times to decide whether we fixed the issue.
//!
//! ### Randomly testing concurrent code with Shuttle
//!
//! Shuttle avoids this issue by controlling the scheduling of each thread in the program, and
//! scheduling those threads *randomly*. By controlling the scheduling, Shuttle allows us to
//! reproduce failing tests deterministically. By using random scheduling, with appropriate
//! heuristics, Shuttle can still catch most (non-adversarial) concurrency bugs even though it is
//! not an exhaustive checker.
//!
//! A Shuttle version of the above test just wraps the test body in a call to Shuttle's
//! [check_random] function, and replaces the concurrency-related imports from `std` with imports
//! from `shuttle`:
//!
//! ```should_panic
//! use shuttle::sync::{Arc, Mutex};
//! use shuttle::thread;
//!
//! shuttle::check_random(|| {
//!     let lock = Arc::new(Mutex::new(0u64));
//!     let lock2 = lock.clone();
//!
//!     thread::spawn(move || {
//!         *lock.lock().unwrap() = 1;
//!     });
//!
//!     assert_eq!(0, *lock2.lock().unwrap());
//! }, 100);
//! ```
//!
//! This test detects the assertion failure with extremely high probability (over 99.9999%).
//!
//! ## Testing non-deterministic code
//!
//! Shuttle supports testing code that uses *data non-determinism* (random number generation). For
//! example, this test uses the [`rand`](https://crates.io/crates/rand) crate to generate a random
//! number:
//!
//! ```no_run
//! use rand::{thread_rng, Rng};
//!
//! let x = thread_rng().gen::<u64>();
//! assert_eq!(x % 10, 7);
//! ```
//!
//! Shuttle provides its own implementation of [`rand`] that is a drop-in replacement:
//!
//! ```should_panic
//! use shuttle::rand::{thread_rng, Rng};
//!
//! shuttle::check_random(|| {
//!     let x = thread_rng().gen::<u64>();
//!     assert_ne!(x % 10, 7);
//! }, 100);
//! ```
//!
//! This test will run the body 100 times, and fail if any of those executions fails; the test
//! therefore fails with probability 1-(9/10)^100, or 99.997%. We can increase the `100` parameter
//! to run more executions and increase the probability of finding the failure. Note that Shuttle
//! isn't doing anything special to increase the probability of this test failing other than running
//! the body multiple times.
//!
//! When this test fails, Shuttle provides output that can be used to **deterministically**
//! reproduce the failure:
//!
//! ```text
//! test panicked in task "task-0" with schedule: "910102ccdedf9592aba2afd70104"
//! pass that schedule string into `shuttle::replay` to reproduce the failure
//! ```
//!
//! We can use Shuttle's [`replay`] function to replay the execution that causes the failure:
//!
//! ```should_panic
//! # // *** DON'T FORGET TO UPDATE THE TEXT OUTPUT RIGHT ABOVE THIS IF YOU CHANGE THIS TEST! ***
//! use shuttle::rand::{thread_rng, Rng};
//!
//! shuttle::replay(|| {
//!     let x = thread_rng().gen::<u64>();
//!     assert_ne!(x % 10, 7);
//! }, "910102ccdedf9592aba2afd70104");
//! ```
//!
//! This runs the test only once, and is guaranteed to reproduce the failure.
//!
//! Support for data non-determinism is most useful when *combined* with support for schedule
//! non-determinism (i.e., concurrency). For example, an integration test might spawn several
//! threads, and within each thread perform a random sequence of actions determined by `thread_rng`
//! (this style of testing is often referred to as a "stress test"). By using Shuttle to implement
//! the stress test, we can both increase the coverage of the test by exploring more thread
//! interleavings and allow test failures to be deterministically reproducible for debugging.
//!
//! ## Writing Shuttle tests
//!
//! To test concurrent code with Shuttle, all uses of synchronization primitives from `std` must be
//! replaced by their Shuttle equivalents. The simplest way to do this is via `cfg` flags.
//! Specifically, if you enforce that all synchronization primitives are imported from a single
//! `sync` module in your code, and implement that module like this:
//!
//! ```
//! #[cfg(all(feature = "shuttle", test))]
//! use shuttle::{sync::*, thread};
//! #[cfg(not(all(feature = "shuttle", test)))]
//! use std::{sync::*, thread};
//! ```
//!
//! Then a Shuttle test can be written like this:
//!
//! ```
//! # mod my_crate {}
//! #[cfg(feature = "shuttle")]
//! #[test]
//! fn concurrency_test_shuttle() {
//!     use my_crate::*;
//!     // ...
//! }
//! ```
//!
//! and be executed by running `cargo test --features shuttle`.
//!
//! ### Choosing a scheduler and running a test
//!
//! Shuttle tests need to choose a *scheduler* to use to direct the execution. The scheduler
//! determines the order in which threads are scheduled. Different scheduling policies can increase
//! the probability of detecting certain classes of bugs (e.g., race conditions), but at the cost of
//! needing to test more executions.
//!
//! Shuttle has a number of built-in schedulers, which implement the
//! [`Scheduler`](crate::scheduler::Scheduler) trait. They are most easily accessed via convenience
//! methods:
//! - [`check_random`] runs a test using a random scheduler for a chosen number of executions.
//! - [`check_pct`] runs a test using the [Probabilistic Concurrency Testing][pct] (PCT) algorithm.
//!   PCT bounds the number of preemptions a test explores; empirically, most concurrency bugs can
//!   be detected with very few preemptions, and so PCT increases the probability of finding such
//!   bugs. The PCT scheduler can be configured with a "bug depth" (the number of preemptions) and a
//!   number of executions.
//! - [`check_dfs`] runs a test with an *exhaustive* scheduler using depth-first search. Exhaustive
//!   testing is intractable for all but the very simplest programs, and so using this scheduler is
//!   not recommended, but it can be useful to thoroughly test small concurrency primitives. The DFS
//!   scheduler can be configured with a bound on the depth of schedules to explore.
//!
//! When these convenience methods do not provide enough control, Shuttle provides a [`Runner`]
//! object for executing a test. A runner is constructed from a chosen [scheduler](scheduler), and
//! then invoked with the [`Runner::run`] method. Shuttle also provides a [`PortfolioRunner`] object
//! for running multiple schedulers, using parallelism to increase the number of test executions
//! explored.
//!
//! [Loom]: https://github.com/tokio-rs/loom
//! [pct]: https://www.microsoft.com/en-us/research/wp-content/uploads/2016/02/asplos277-pct.pdf

pub mod future;
pub mod hint;
pub mod rand;
pub mod sync;
pub mod thread;

pub mod current;
pub mod scheduler;

pub mod runtime;

pub use scheduler::fuzz::{CompletionMode};

use std::panic::{RefUnwindSafe, UnwindSafe};

pub use runtime::runner::{PortfolioRunner, Runner};
// use scheduler::CompletionMode;

/// Configuration parameters for Shuttle
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct Config {
    /// Stack size allocated for each thread
    pub stack_size: usize,

    /// How to persist schedules when a test fails
    pub failure_persistence: FailurePersistence,

    /// Maximum number of steps a single iteration of a test can take, and how to react when the
    /// limit is reached
    pub max_steps: MaxSteps,

    /// Time limit for an entire test. If set, calls to [`Runner::run`] will return when the time
    /// limit is exceeded or the [`Scheduler`](crate::scheduler::Scheduler) chooses to stop (e.g.,
    /// by hitting its maximum number of iterations), whichever comes first. This time limit will
    /// not abort a currently running test iteration; the limit is only checked between iterations.
    pub max_time: Option<std::time::Duration>,

    /// Whether to enable warnings about [Shuttle's unsound implementation of
    /// `atomic`](crate::sync::atomic#warning-about-relaxed-behaviors).
    pub silence_atomic_ordering_warning: bool,
}

impl Config {
    /// Create a new default configuration
    pub fn new() -> Self {
        Self {
            stack_size: 0x8000,
            failure_persistence: FailurePersistence::Print,
            max_steps: MaxSteps::FailAfter(1_000_000),
            max_time: None,
            silence_atomic_ordering_warning: false,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::new()
    }
}

/// Specifies how to persist schedules when a Shuttle test fails
///
/// By default, schedules are printed to stdout/stderr, and can be replayed using [`replay`].
/// Optionally, they can instead be persisted to a file and replayed using [`replay_from_file`],
/// which can be useful if the schedule is too large to conveniently include in a call to
/// [`replay`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum FailurePersistence {
    /// Do not persist failing schedules
    None,
    /// Print failing schedules to stdout/stderr
    Print,
    /// Persist schedules as files in the given directory, or the current directory if None.
    File(Option<std::path::PathBuf>),
}

/// Specifies an upper bound on the number of steps a single iteration of a Shuttle test can take,
/// and how to react when the bound is reached.
///
/// A "step" is an atomic region (all the code between two yieldpoints). For example, all the
/// (non-concurrency-operation) code between acquiring and releasing a [`Mutex`] is a single step.
/// Shuttle can bound the maximum number of steps a single test iteration can take to prevent
/// infinite loops. If the bound is hit, the test can either fail (`FailAfter`) or continue to the
/// next iteration (`ContinueAfter`).
///
/// The steps bound can be used to protect against livelock and fairness issues. For example, if a
/// thread is waiting for another thread to make progress, but the chosen [`Scheduler`] never
/// schedules that thread, a livelock occurs and the test will not terminate without a step bound.
///
/// By default, Shuttle fails a test after 1,000,000 steps.
///
/// [`Mutex`]: crate::sync::Mutex
/// [`Scheduler`]: crate::scheduler::Scheduler
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum MaxSteps {
    /// Do not enforce any bound on the maximum number of steps
    None,
    /// Fail the test (by panicing) after the given number of steps
    FailAfter(usize),
    /// When the given number of steps is reached, stop the current iteration of the test and
    /// begin a new iteration
    ContinueAfter(usize),
}

/// Run the given function once under a round-robin concurrency scheduler.
// TODO consider removing this -- round robin scheduling is never what you want.
#[doc(hidden)]
pub fn check<F>(f: F)
where
    F: Fn() + Send + Sync + 'static,
{
    use crate::scheduler::RoundRobinScheduler;

    let runner = Runner::new(RoundRobinScheduler::new(), Default::default());
    runner.run(f);
}

/// Run the given function under a randomized concurrency scheduler for some number of iterations.
/// Each iteration will run a (potentially) different randomized schedule.
pub fn check_random<F>(f: F, iterations: usize)
where
    F: Fn() + Send + Sync + 'static,
{
    use crate::scheduler::RandomScheduler;
    tracing::info!("checking random");
    let scheduler = RandomScheduler::new(iterations);
    let runner = Runner::new(scheduler, Default::default());
    runner.run(f);
}


pub fn check_fuzz<F>(f: F, _iterations: usize, mode: CompletionMode)
where
    F: Fn() + Send + Sync + RefUnwindSafe + UnwindSafe + 'static,
{
    use crate::scheduler::{FuzzScheduler, CompletionMode};

    let scheduler = FuzzScheduler::new(mode);
    let runner = Runner::new(scheduler, Default::default());
    runner.run_fuzz(f);
}

pub fn check_pct_fuzz<F>(f: F, iterations: usize, depth: usize)
where
    F: Fn() + Send + Sync + RefUnwindSafe + UnwindSafe + 'static + Copy,
{
    use crate::scheduler::FuzzPctScheduler;
    use afl::fuzz;


    let scheduler = FuzzPctScheduler::new(depth, iterations);
    let runner = Runner::new(scheduler, Default::default());
    runner.run_pct_fuzz(f);
}

pub fn check_pct_fuzz_dumb<F>(f: F)
where
    F: Fn() + Send + Sync + RefUnwindSafe + UnwindSafe + 'static + Copy,
{
    use crate::scheduler::PctScheduler;
    use afl::fuzz;
    use arbitrary::Arbitrary;
    use std::sync::Arc;

    #[derive(Clone, Debug, Default, PartialEq, Eq, Arbitrary)]
    struct PctParams {
        max_depth: usize,
        // not sure if we want to do this.. 
        // may want to set this and just do it "enough" times? hmm
        max_iterations: usize,
    }

    let f = Arc::new(f);
    let mut counter = 0;
    fuzz!(|p: PctParams| {
        tracing::info!("round {:?}, fuzzing {:?}", counter, p);
        let f = Arc::clone(&f);
        let scheduler = PctScheduler::new(p.max_depth, p.max_iterations);
        let runner = Runner::new(scheduler, Default::default());
        runner.run(*f);
        counter += 1;
    });
    
}

/// Run the given function under a PCT concurrency scheduler for some number of iterations at the
/// given depth. Each iteration will run a (potentially) different randomized schedule.
pub fn check_pct<F>(f: F, iterations: usize, depth: usize)
where
    F: Fn() + Send + Sync + 'static,
{
    use crate::scheduler::PctScheduler;

    let scheduler = PctScheduler::new(depth, iterations);
    let runner = Runner::new(scheduler, Default::default());
    runner.run(f);
}

/// Run the given function under a depth-first-search scheduler until all interleavings have been
/// explored (but if the max_iterations bound is provided, stop after that many iterations).
pub fn check_dfs<F>(f: F, max_iterations: Option<usize>)
where
    F: Fn() + Send + Sync + 'static,
{
    use crate::scheduler::DfsScheduler;

    let scheduler = DfsScheduler::new(max_iterations, false);
    let runner = Runner::new(scheduler, Default::default());
    runner.run(f);
}

/// Run the given function according to a given encoded schedule, usually produced as the output of
/// a failing Shuttle test case.
///
/// This function allows deterministic replay of a failing schedule, as long as `f` contains no
/// non-determinism other than that introduced by scheduling.
///
/// This is a convenience function for constructing a [`Runner`] that uses
/// [`ReplayScheduler::new_from_encoded`](scheduler::ReplayScheduler::new_from_encoded).
pub fn replay<F>(f: F, encoded_schedule: &str)
where
    F: Fn() + Send + Sync + 'static,
{
    use crate::scheduler::ReplayScheduler;

    let scheduler = ReplayScheduler::new_from_encoded(encoded_schedule);
    let runner = Runner::new(scheduler, Default::default());
    runner.run(f);
}

/// Run the given function according to a schedule saved in the given file, usually produced as the
/// output of a failing Shuttle test case.
///
/// This function allows deterministic replay of a failing schedule, as long as `f` contains no
/// non-determinism other than that introduced by scheduling.
///
/// This is a convenience function for constructing a [`Runner`] that uses
/// [`ReplayScheduler::new_from_file`](scheduler::ReplayScheduler::new_from_file).
pub fn replay_from_file<F, P>(f: F, path: P)
where
    F: Fn() + Send + Sync + 'static,
    P: AsRef<std::path::Path>,
{
    use crate::scheduler::ReplayScheduler;

    let scheduler = ReplayScheduler::new_from_file(path).expect("could not load schedule from file");
    let runner = Runner::new(scheduler, Default::default());
    runner.run(f);
}

/// Declare a new thread local storage key of type [`LocalKey`](crate::thread::LocalKey).
#[macro_export]
macro_rules! thread_local {
    // empty (base case for the recursion)
    () => {};

    // process multiple declarations with a const initializer
    ($(#[$attr:meta])* $vis:vis static $name:ident: $t:ty = const { $init:expr }; $($rest:tt)*) => (
        $crate::__thread_local_inner!($(#[$attr])* $vis $name, $t, $init);
        $crate::thread_local!($($rest)*);
    );

    // handle a single declaration with a const initializer
    ($(#[$attr:meta])* $vis:vis static $name:ident: $t:ty = const { $init:expr }) => (
        $crate::__thread_local_inner!($(#[$attr])* $vis $name, $t, $init);
    );

    // process multiple declarations
    ($(#[$attr:meta])* $vis:vis static $name:ident: $t:ty = $init:expr; $($rest:tt)*) => (
        $crate::__thread_local_inner!($(#[$attr])* $vis $name, $t, $init);
        $crate::thread_local!($($rest)*);
    );

    // handle a single declaration
    ($(#[$attr:meta])* $vis:vis static $name:ident: $t:ty = $init:expr) => (
        $crate::__thread_local_inner!($(#[$attr])* $vis $name, $t, $init);
    );
}

#[doc(hidden)]
#[macro_export]
macro_rules! __thread_local_inner {
    ($(#[$attr:meta])* $vis:vis $name:ident, $t:ty, $init:expr) => {
        $(#[$attr])* $vis static $name: $crate::thread::LocalKey<$t> =
            $crate::thread::LocalKey {
                init: || { $init },
                _p: std::marker::PhantomData,
            };
    }
}
