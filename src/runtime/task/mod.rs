use crate::runtime::execution::ExecutionState;
use crate::runtime::storage::{AlreadyDestructedError, StorageKey, StorageMap};
use crate::runtime::task::clock::VectorClock;
use crate::runtime::thread;
use crate::runtime::thread::continuation::{ContinuationPool, PooledContinuation};
use crate::thread::LocalKey;
use bitvec::prelude::*;
use std::any::Any;
use std::cell::RefCell;
use std::fmt::Debug;
use std::future::Future;
use std::rc::Rc;
use std::task::{Context, Waker};

pub(crate) mod clock;
pub(crate) mod waker;
use waker::make_waker;

// A note on terminology: we have competing notions of threads floating around. Here's the
// convention for disambiguating them:
// * A "thread" is a user-level unit of concurrency. User code creates threads, passes data
//   between them, etc.
// * A "future" is another user-level unit of concurrency, corresponding directly to Rust's notion
//   in std::future::Future. A future has a single method `poll` that can be used to resume
//   executing its computation. Both futures and threads are implemented in Task,
//   which wraps a continuation that is resumed when the task is scheduled.
// * A "task" is the Shuttle executor's reflection of a user-level unit of concurrency. Each task
//   has a corresponding continuation, which is the user-level code it runs, as well as a state like
//   "blocked", "runnable", etc. Scheduling algorithms take as input the state of all tasks
//   and decide which task should execute next. A context switch is when one task stops executing
//   and another begins.
// * A "continuation" is a low-level implementation of green threading for concurrency. Each
//   Task contains a corresponding continuation. When the Shuttle executor context switches to a
//   Task, the executor resumes that task's continuation until it yields, which happens when its
//   thread decides it might want to context switch (e.g., because it's blocked on a lock).

pub(crate) const DEFAULT_INLINE_TASKS: usize = 16;

/// A `Task` represents a user-level unit of concurrency. Each task has an `id` that is unique within
/// the execution, and a `state` reflecting whether the task is runnable (enabled) or not.
#[derive(Debug)]
pub(crate) struct Task {
    pub(super) id: TaskId,
    pub(super) state: TaskState,
    pub(super) detached: bool,
    park_state: ParkState,

    pub(super) continuation: Rc<RefCell<PooledContinuation>>,

    pub(crate) clock: VectorClock,

    waiter: Option<TaskId>,

    waker: Waker,
    // Remember whether the waker was invoked while we were running
    woken: bool,

    name: Option<String>,

    local_storage: StorageMap,
}

impl Task {
    /// Create a task from a continuation
    fn new<F>(f: F, stack_size: usize, id: TaskId, name: Option<String>, clock: VectorClock) -> Self
    where
        F: FnOnce() + Send + 'static,
    {
        assert!(id.0 < clock.time.len());
        let mut continuation = ContinuationPool::acquire(stack_size);
        continuation.initialize(Box::new(f));
        let waker = make_waker(id);
        let continuation = Rc::new(RefCell::new(continuation));
        Self {
            id,
            state: TaskState::Runnable,
            continuation,
            clock,
            waiter: None,
            waker,
            woken: false,
            detached: false,
            park_state: ParkState::Unavailable,
            name,
            local_storage: StorageMap::new(),
        }
    }

    pub(crate) fn from_closure<F>(f: F, stack_size: usize, id: TaskId, name: Option<String>, clock: VectorClock) -> Self
    where
        F: FnOnce() + Send + 'static,
    {
        Self::new(f, stack_size, id, name, clock)
    }

    pub(crate) fn from_future<F>(
        future: F,
        stack_size: usize,
        id: TaskId,
        name: Option<String>,
        clock: VectorClock,
    ) -> Self
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let mut future = Box::pin(future);
        Self::new(
            move || {
                let waker = ExecutionState::with(|state| state.current_mut().waker());
                let cx = &mut Context::from_waker(&waker);
                while future.as_mut().poll(cx).is_pending() {
                    ExecutionState::with(|state| state.current_mut().sleep_unless_woken());
                    thread::switch();
                }
            },
            stack_size,
            id,
            name,
            clock,
        )
    }

    pub(crate) fn id(&self) -> TaskId {
        self.id
    }

    pub(crate) fn runnable(&self) -> bool {
        self.state == TaskState::Runnable
    }

    pub(crate) fn blocked(&self) -> bool {
        self.state == TaskState::Blocked
    }

    pub(crate) fn sleeping(&self) -> bool {
        self.state == TaskState::Sleeping
    }

    pub(crate) fn finished(&self) -> bool {
        self.state == TaskState::Finished
    }

    pub(crate) fn detach(&mut self) {
        self.detached = true;
    }

    pub(crate) fn waker(&self) -> Waker {
        self.waker.clone()
    }

    pub(crate) fn block(&mut self) {
        assert!(self.state != TaskState::Finished);
        self.state = TaskState::Blocked;
    }

    pub(crate) fn sleep(&mut self) {
        assert!(self.state != TaskState::Finished);
        self.state = TaskState::Sleeping;
    }

    pub(crate) fn unblock(&mut self) {
        // Note we don't assert the task is blocked here. For example, a task invoking its own waker
        // will not be blocked when this is called.
        assert!(self.state != TaskState::Finished);
        self.state = TaskState::Runnable;
    }

    pub(crate) fn finish(&mut self) {
        assert!(self.state != TaskState::Finished);
        self.state = TaskState::Finished;
    }

    /// Potentially put this task to sleep after it was polled by the executor, unless someone has
    /// called its waker first.
    ///
    /// A synchronous Task should never call this, because we want threads to be enabled-by-default
    /// to avoid bugs where Shuttle incorrectly omits a potential execution.
    pub(crate) fn sleep_unless_woken(&mut self) {
        let was_woken = std::mem::replace(&mut self.woken, false);
        if !was_woken {
            self.sleep();
        }
    }

    /// Remember that our waker has been called, and so we should not block the next time the
    /// executor tries to put us to sleep.
    pub(super) fn wake(&mut self) {
        self.woken = true;
        if self.state == TaskState::Sleeping {
            self.unblock();
        }
    }

    /// Register a waiter for this thread to terminate. Returns a boolean indicating whether the
    /// waiter should block or not. If false, this task has already finished, and so the waiter need
    /// not block.
    pub(crate) fn set_waiter(&mut self, waiter: TaskId) -> bool {
        assert!(
            self.waiter.is_none() || self.waiter == Some(waiter),
            "Task cannot have more than one waiter"
        );
        if self.finished() {
            false
        } else {
            self.waiter = Some(waiter);
            true
        }
    }

    pub(crate) fn take_waiter(&mut self) -> Option<TaskId> {
        self.waiter.take()
    }

    pub(crate) fn name(&self) -> Option<String> {
        self.name.clone()
    }

    /// Retrieve a reference to the given thread-local storage slot.
    ///
    /// Returns Some(Err(_)) if the slot has already been destructed. Returns None if the slot has
    /// not yet been initialized.
    pub(crate) fn local<T: 'static>(&self, key: &'static LocalKey<T>) -> Option<Result<&T, AlreadyDestructedError>> {
        self.local_storage.get(key.into())
    }

    /// Initialize the given thread-local storage slot with a new value.
    ///
    /// Panics if the slot has already been initialized.
    pub(crate) fn init_local<T: 'static>(&mut self, key: &'static LocalKey<T>, value: T) {
        self.local_storage.init(key.into(), value)
    }

    /// Return ownership of the next still-initialized thread-local storage slot, to be used when
    /// running thread-local storage destructors.
    ///
    /// TLS destructors are a little tricky:
    /// 1. Their code can perform synchronization operations (and so require Shuttle to call back
    ///    into ExecutionState), so we can't drop them from within an ExecutionState borrow. Instead
    ///    we move the contents of a slot to the caller to be dropped outside the borrow.
    /// 2. It's valid for destructors to read other TLS slots, although destructor order is
    ///    undefined. This also means it's valid for a destructor to *initialize* another TLS slot.
    ///    To make this work, we run the destructors incrementally, so one destructor can initialize
    ///    another slot that just gets added via `init_local` like normal, and then will be
    ///    available to be popped on a future call to `pop_local`. To prevent an infinite loop, we
    ///    forbid *reinitializing* a TLS slot whose destructor has already run, or is currently
    ///    being run.
    pub(crate) fn pop_local(&mut self) -> Option<Box<dyn Any>> {
        self.local_storage.pop()
    }

    /// Park the task if its park token is unavailable. Returns true if the token was unavailable.
    pub(crate) fn park(&mut self) -> bool {
        match self.park_state {
            ParkState::Unparked => {
                self.park_state = ParkState::Unavailable;
                false
            }
            ParkState::Unavailable => {
                self.park_state = ParkState::Parked;
                self.block();
                true
            }
            ParkState::Parked => unreachable!("cannot park a task that's already parked"),
        }
    }

    /// Make the task's park token available, and unblock the task if it was parked.
    pub(crate) fn unpark(&mut self) {
        if std::mem::replace(&mut self.park_state, ParkState::Unparked) == ParkState::Parked {
            assert!(self.blocked());
            self.unblock();
        }
    }
}

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub(crate) enum TaskState {
    /// Available to be scheduled
    Runnable,
    /// Blocked in a synchronization operation
    Blocked,
    /// A `Future` that returned `Pending` is waiting to be woken up
    Sleeping,
    /// Task has finished
    Finished,
}

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub(crate) enum ParkState {
    /// The task has parked itself and not yet been unparked, so the park token is unavailable.
    /// Invariant: if ParkState is Parked, the task is Blocked
    Parked,
    /// Another task has unparked this one, so the park token is available.
    Unparked,
    /// The park token is not available. The task should enter Parked state on the next `park` call.
    Unavailable,
}

/// A `TaskId` is a unique identifier for a task. `TaskId`s are never reused within a single
/// execution.
use arbitrary::Arbitrary;
#[derive(PartialEq, Eq, Hash, Clone, Copy, PartialOrd, Ord, Debug, Arbitrary)]
pub struct TaskId(pub(super) usize);

impl From<usize> for TaskId {
    fn from(id: usize) -> Self {
        TaskId(id)
    }
}

impl From<TaskId> for usize {
    fn from(tid: TaskId) -> usize {
        tid.0
    }
}

/// A `TaskSet` is a set of `TaskId`s but implemented efficiently as an array of bools.
// TODO this probably won't work well with large numbers of tasks -- maybe a BitVec?
#[derive(PartialEq, Eq)]
pub(crate) struct TaskSet {
    tasks: BitVec,
}

impl TaskSet {
    pub fn new() -> Self {
        Self {
            tasks: BitVec::from_bitslice(bits![0; DEFAULT_INLINE_TASKS]),
        }
    }

    pub fn contains(&self, tid: TaskId) -> bool {
        // Return false if tid is outside the TaskSet
        (tid.0 < self.tasks.len()) && self.tasks[tid.0]
    }

    pub fn is_empty(&self) -> bool {
        self.tasks.iter().all(|b| !*b)
    }

    /// Add a task to the set. If the set did not have this value present, `true` is returned. If
    /// the set did have this value present, `false` is returned.
    pub fn insert(&mut self, tid: TaskId) -> bool {
        if tid.0 >= self.tasks.len() {
            self.tasks.resize(1 + tid.0, false);
        }
        !std::mem::replace(&mut *self.tasks.get_mut(tid.0).unwrap(), true)
    }

    /// Removes a value from the set. Returns whether the value was present in the set.
    pub fn remove(&mut self, tid: TaskId) -> bool {
        if tid.0 >= self.tasks.len() {
            return false;
        }
        std::mem::replace(&mut self.tasks.get_mut(tid.0).unwrap(), false)
    }

    pub fn iter(&self) -> impl Iterator<Item = TaskId> + '_ {
        self.tasks
            .iter()
            .enumerate()
            .filter(|(_, b)| **b)
            .map(|(i, _)| TaskId(i))
    }
}

impl Debug for TaskSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TaskSet {{ ")?;
        for t in self.iter() {
            write!(f, "{} ", t.0)?;
        }
        write!(f, "}}")
    }
}

impl<T: 'static> From<&'static LocalKey<T>> for StorageKey {
    fn from(key: &'static LocalKey<T>) -> Self {
        Self(key as *const _ as usize, 0x1)
    }
}
