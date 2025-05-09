//! Simulation halt and resume.
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use nexosim::model::{Context, InitializedModel, Model};
use nexosim::ports::{EventQueue, Output};
use nexosim::simulation::{ExecutionError, Mailbox, SimInit, SimulationError};
use nexosim::time::{MonotonicTime, SystemClock};

struct RecurringModel {
    pub message: Output<()>,
    delay: Duration,
}
impl RecurringModel {
    fn new(delay: Duration) -> Self {
        Self {
            delay,
            message: Output::new(),
        }
    }
    async fn process(&mut self) {
        self.message.send(()).await;
    }
}
impl Model for RecurringModel {
    async fn init(self, cx: &mut Context<Self>) -> InitializedModel<Self> {
        cx.schedule_periodic_event(self.delay, self.delay, RecurringModel::process, ())
            .unwrap();

        self.into()
    }
}

#[test]
fn halt_and_resume() -> Result<(), SimulationError> {
    let mut model = RecurringModel::new(Duration::from_millis(200));
    let mailbox = Mailbox::new();

    let message = EventQueue::new();
    model.message.connect_sink(&message);
    let mut message = message.into_reader();

    let t0 = MonotonicTime::EPOCH;

    let (simu, mut scheduler) = SimInit::new()
        .add_model(model, mailbox, "timed_model")
        .set_clock(SystemClock::from_instant(t0, Instant::now()))
        .init(t0)?;

    let simulation = Arc::new(Mutex::new(simu));
    let spawned_simulation = simulation.clone();

    let simulation_handle =
        thread::spawn(move || spawned_simulation.lock().unwrap().step_unbounded());

    thread::sleep(Duration::from_millis(100)); // pause until t(sim) = t(wall clock) = t0 + delta

    scheduler.halt();
    thread::sleep(Duration::from_millis(200)); // pause until t(wall clock) = t0 + 3*delta

    // Assert that the step has completed at t = t0 + 2*delta, even though
    // `halt` was called before its scheduled time.
    assert!(message.next().is_some());

    match simulation_handle.join().unwrap() {
        Err(ExecutionError::Halted) => (),
        Err(e) => return Err(e.into()),
        _ => (),
    };

    thread::sleep(Duration::from_millis(200)); // pause until t(wall clock) = t0 + 5*delta

    // Halted - no new messages.
    assert!(message.next().is_none());

    // Restart the simulation.
    {
        let mut s = simulation.try_lock().unwrap();
        let t1 = s.time();

        // The simulation should have stopped just after the first scheduled event.
        assert_eq!(t1, t0 + Duration::from_millis(200));

        // Restart the simulation at the last simulation time.
        s.reset_clock(SystemClock::from_instant(t1, Instant::now()));
    }
    let spawned_simulation = simulation.clone();
    let simulation_handle =
        thread::spawn(move || spawned_simulation.lock().unwrap().step_unbounded());

    thread::sleep(Duration::from_millis(100)); // pause until t(sim) = t(wall clock) = t1 + delta

    // No new messages yet, as the halt time should be invisible to the
    // scheduler.
    assert!(message.next().is_none());

    thread::sleep(Duration::from_millis(200)); // pause until t(wall clock) = t1 + 3*delta
    assert!(message.next().is_some());

    scheduler.halt();

    match simulation_handle.join().unwrap() {
        Err(ExecutionError::Halted) => Ok(()),
        Err(e) => Err(e.into()),
        _ => Ok(()),
    }
}
