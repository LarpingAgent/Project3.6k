use std::{cell::RefCell, rc::Rc, time::Duration};

use project3_6k_engine::{EngineConfig, EngineEvent, EngineRuntime, FixedUpdate, VariableUpdate};

#[test]
fn host_runs_start_event_update_render_and_orderly_shutdown() {
    let start = std::time::Instant::now();
    let mut engine = EngineRuntime::new(EngineConfig {
        fixed_update_interval: Duration::from_millis(10),
        max_frame_delta: Duration::from_millis(100),
        max_fixed_updates_per_frame: 4,
        event_queue_capacity: 16,
    })
    .unwrap();
    let order = Rc::new(RefCell::new(Vec::new()));
    engine.start_at(start).unwrap();
    order.borrow_mut().push("start");
    engine
        .queue_event(EngineEvent::WindowResized {
            width: 1280,
            height: 720,
        })
        .unwrap();

    let event_order = order.clone();
    let fixed_order = order.clone();
    let update_order = order.clone();
    let schedule = engine
        .run_frame(
            start + Duration::from_millis(20),
            move |_| event_order.borrow_mut().push("event"),
            move |_: FixedUpdate| fixed_order.borrow_mut().push("fixed update"),
            move |_: VariableUpdate| update_order.borrow_mut().push("variable update"),
        )
        .unwrap();
    assert_eq!(schedule.events_dispatched, 1);
    assert_eq!(schedule.fixed_updates, 2);
    order.borrow_mut().push("render");

    engine.request_shutdown().unwrap();
    engine.shutdown();
    order.borrow_mut().push("shutdown");

    assert_eq!(
        *order.borrow(),
        [
            "start",
            "event",
            "fixed update",
            "fixed update",
            "variable update",
            "render",
            "shutdown",
        ]
    );
}
