use super::{super::ScaleChangeArgs, backend, state::State};
use crate::event::{Event, StartCause};
use crate::event_loop as root;
use crate::window::WindowId;

use instant::{Duration, Instant};
use std::{
    cell::RefCell,
    clone::Clone,
    collections::{HashSet, VecDeque},
    iter,
    rc::Rc,
};

pub struct Shared<T: 'static>(Rc<Execution<T>>);

impl<T> Clone for Shared<T> {
    fn clone(&self) -> Self {
        Shared(self.0.clone())
    }
}

pub struct Execution<T: 'static> {
    runner: RefCell<Option<Runner<T>>>,
    events: RefCell<VecDeque<Event<'static, T>>>,
    id: RefCell<u32>,
    all_canvases: RefCell<Vec<(WindowId, backend::RawCanvasType)>>,
    redraw_pending: RefCell<HashSet<WindowId>>,
    scale_change_detector: RefCell<Option<backend::ScaleChangeDetector>>,
}

struct Runner<T: 'static> {
    state: State,
    is_busy: bool,
    event_handler: Box<dyn FnMut(Event<'_, T>, &mut root::ControlFlow)>,
}

impl<T: 'static> Runner<T> {
    pub fn new(event_handler: Box<dyn FnMut(Event<'_, T>, &mut root::ControlFlow)>) -> Self {
        Runner {
            state: State::Init,
            is_busy: false,
            event_handler,
        }
    }

    /// Returns the cooresponding `StartCause` for the current `state`, or `None`
    /// when in `Exit` state.
    fn maybe_start_cause(&self) -> Option<StartCause> {
        Some(match self.state {
            State::Init => StartCause::Init,
            State::Poll { .. } => StartCause::Poll,
            State::Wait { start } => StartCause::WaitCancelled {
                start,
                requested_resume: None,
            },
            State::WaitUntil { start, end, .. } => StartCause::WaitCancelled {
                start,
                requested_resume: Some(end),
            },
            State::Exit => return None,
        })
    }

    fn handle_single_event(&mut self, event: Event<'_, T>, control: &mut root::ControlFlow) {
        let is_closed = *control == root::ControlFlow::Exit;

        // An event is being processed, so the runner should be marked busy
        self.is_busy = true;

        (self.event_handler)(event, control);

        // Maintain closed state, even if the callback changes it
        if is_closed {
            *control = root::ControlFlow::Exit;
        }

        // An event is no longer being processed
        self.is_busy = false;
    }
}

impl<T: 'static> Shared<T> {
    pub fn new() -> Self {
        Shared(Rc::new(Execution {
            runner: RefCell::new(None),
            events: RefCell::new(VecDeque::new()),
            id: RefCell::new(0),
            all_canvases: RefCell::new(Vec::new()),
            redraw_pending: RefCell::new(HashSet::new()),
            scale_change_detector: RefCell::new(None),
        }))
    }

    pub fn add_canvas(&self, id: WindowId, canvas: backend::RawCanvasType) {
        self.0.all_canvases.borrow_mut().push((id, canvas));
    }

    // Set the event callback to use for the event loop runner
    // This the event callback is a fairly thin layer over the user-provided callback that closes
    // over a RootEventLoopWindowTarget reference
    pub fn set_listener(
        &self,
        event_handler: Box<dyn FnMut(Event<'_, T>, &mut root::ControlFlow)>,
    ) {
        self.0.runner.replace(Some(Runner::new(event_handler)));
        self.init();

        let close_instance = self.clone();
        backend::on_unload(move || close_instance.handle_unload());
    }

    pub(crate) fn set_on_scale_change<F>(&self, handler: F)
    where
        F: 'static + FnMut(ScaleChangeArgs),
    {
        *self.0.scale_change_detector.borrow_mut() =
            Some(backend::ScaleChangeDetector::new(handler));
    }

    // Generate a strictly increasing ID
    // This is used to differentiate windows when handling events
    pub fn generate_id(&self) -> u32 {
        let mut id = self.0.id.borrow_mut();
        *id += 1;

        *id
    }

    pub fn request_redraw(&self, id: WindowId) {
        self.0.redraw_pending.borrow_mut().insert(id);
    }

    pub fn init(&self) {
        let start_cause = Event::NewEvents(StartCause::Init);
        self.run_until_cleared(iter::once(start_cause));
    }

    // Run the polling logic for the Poll ControlFlow, which involves clearing the queue
    pub fn poll(&self) {
        let start_cause = Event::NewEvents(StartCause::Poll);
        self.run_until_cleared(iter::once(start_cause));
    }

    // Run the logic for waking from a WaitUntil, which involves clearing the queue
    // Generally there shouldn't be events built up when this is called
    pub fn resume_time_reached(&self, start: Instant, requested_resume: Instant) {
        let start_cause = Event::NewEvents(StartCause::ResumeTimeReached {
            start,
            requested_resume,
        });
        self.run_until_cleared(iter::once(start_cause));
    }

    // Add an event to the event loop runner, from the user or an event handler
    //
    // It will determine if the event should be immediately sent to the user or buffered for later
    pub fn send_event(&self, event: Event<'static, T>) {
        self.send_events(iter::once(event));
    }

    // Add a series of events to the event loop runner
    //
    // It will determine if the event should be immediately sent to the user or buffered for later
    pub fn send_events(&self, events: impl Iterator<Item = Event<'static, T>>) {
        // If the event loop is closed, it should discard any new events
        if self.is_closed() {
            return;
        }
        // If we can run the event processing right now, or need to queue this and wait for later
        let mut process_immediately = true;
        if let Some(ref runner) = &*self.0.runner.borrow() {
            // If we're currently polling, queue this and wait for the poll() method to be called
            if let State::Poll { .. } = runner.state {
                process_immediately = false;
            }
            // If the runner is busy, queue this and wait for it to process it later
            if runner.is_busy {
                process_immediately = false;
            }
        } else {
            // The runner still hasn't been attached: queue this event and wait for it to be
            process_immediately = false;
        }
        if !process_immediately {
            // Queue these events to look at later
            self.0.events.borrow_mut().extend(events);
            return;
        }
        // At this point, we know this is a fresh set of events
        // Now we determine why new events are incoming, and handle the events
        let start_cause = match (self.0.runner.borrow().as_ref())
            .unwrap_or_else(|| {
                unreachable!("The runner cannot process events when it is not attached")
            })
            .maybe_start_cause()
        {
            Some(c) => c,
            // If we're in the exit state, don't do event processing
            None => return,
        };
        // Take the start event, then the events provided to this function, and run an iteration of
        // the event loop
        let start_event = Event::NewEvents(start_cause);
        let events = iter::once(start_event).chain(events);
        self.run_until_cleared(events);
    }

    // Given the set of new events, run the event loop until the main events and redraw events are
    // cleared
    //
    // This will also process any events that have been queued or that are queued during processing
    fn run_until_cleared(&self, events: impl Iterator<Item = Event<'static, T>>) {
        let mut control = self.current_control_flow();
        for event in events {
            self.handle_event(event, &mut control);
        }
        self.handle_event(Event::MainEventsCleared, &mut control);

        // Collect all of the redraw events to avoid double-locking the RefCell
        let redraw_events: Vec<WindowId> = self.0.redraw_pending.borrow_mut().drain().collect();
        for window_id in redraw_events {
            self.handle_event(Event::RedrawRequested(window_id), &mut control);
        }
        self.handle_event(Event::RedrawEventsCleared, &mut control);

        self.apply_control_flow(control);
        // If the event loop is closed, it has been closed this iteration and now the closing
        // event should be emitted
        if self.is_closed() {
            self.handle_event(Event::LoopDestroyed, &mut control);
        }
    }

    pub fn handle_scale_changed(&self, old_scale: f64, new_scale: f64) {
        let start_cause = match (self.0.runner.borrow().as_ref())
            .unwrap_or_else(|| unreachable!("`scale_changed` should not happen without a runner"))
            .maybe_start_cause()
        {
            Some(c) => c,
            // If we're in the exit state, don't do event processing
            None => return,
        };
        let mut control = self.current_control_flow();

        // Handle the start event and all other events in the queue.
        self.handle_event(Event::NewEvents(start_cause), &mut control);

        // Now handle the `ScaleFactorChanged` events.
        for &(id, ref canvas) in &*self.0.all_canvases.borrow() {
            // First, we send the `ScaleFactorChanged` event:
            let current_size = crate::dpi::PhysicalSize {
                width: canvas.width() as u32,
                height: canvas.height() as u32,
            };
            let logical_size = current_size.to_logical::<f64>(old_scale);
            let mut new_size = logical_size.to_physical(new_scale);
            self.handle_single_event_sync(
                Event::WindowEvent {
                    window_id: id,
                    event: crate::event::WindowEvent::ScaleFactorChanged {
                        scale_factor: new_scale,
                        new_inner_size: &mut new_size,
                    },
                },
                &mut control,
            );

            // Then we resize the canvas to the new size and send a `Resized` event:
            backend::set_canvas_size(canvas, crate::dpi::Size::Physical(new_size));
            self.handle_single_event_sync(
                Event::WindowEvent {
                    window_id: id,
                    event: crate::event::WindowEvent::Resized(new_size),
                },
                &mut control,
            );
        }

        self.handle_event(Event::MainEventsCleared, &mut control);

        // Discard all the pending redraw as we shall just redraw all windows.
        self.0.redraw_pending.borrow_mut().clear();
        for &(window_id, _) in &*self.0.all_canvases.borrow() {
            self.handle_event(Event::RedrawRequested(window_id), &mut control);
        }
        self.handle_event(Event::RedrawEventsCleared, &mut control);

        self.apply_control_flow(control);
        // If the event loop is closed, it has been closed this iteration and now the closing
        // event should be emitted
        if self.is_closed() {
            self.handle_event(Event::LoopDestroyed, &mut control);
        }
    }

    fn handle_unload(&self) {
        self.apply_control_flow(root::ControlFlow::Exit);
        let mut control = self.current_control_flow();
        self.handle_event(Event::LoopDestroyed, &mut control);
    }

    // handle_single_event_sync takes in an event and handles it synchronously.
    //
    // It should only ever be called from `scale_changed`.
    fn handle_single_event_sync(&self, event: Event<'_, T>, control: &mut root::ControlFlow) {
        if self.is_closed() {
            *control = root::ControlFlow::Exit;
        }
        match *self.0.runner.borrow_mut() {
            Some(ref mut runner) => {
                runner.handle_single_event(event, control);
            }
            _ => panic!("Cannot handle event synchronously without a runner"),
        }
    }

    // handle_event takes in events and either queues them or applies a callback
    //
    // It should only ever be called from `run_until_cleared` and `scale_changed`.
    fn handle_event(&self, event: Event<'static, T>, control: &mut root::ControlFlow) {
        if self.is_closed() {
            *control = root::ControlFlow::Exit;
        }
        match *self.0.runner.borrow_mut() {
            Some(ref mut runner) => {
                runner.handle_single_event(event, control);
            }
            // If an event is being handled without a runner somehow, add it to the event queue so
            // it will eventually be processed
            _ => self.0.events.borrow_mut().push_back(event),
        }

        let is_closed = *control == root::ControlFlow::Exit;

        // Don't take events out of the queue if the loop is closed or the runner doesn't exist
        // If the runner doesn't exist and this method recurses, it will recurse infinitely
        if !is_closed && self.0.runner.borrow().is_some() {
            // Take an event out of the queue and handle it
            // Make sure not to let the borrow_mut live during the next handle_event

            let event = { self.0.events.borrow_mut().pop_front() };
            if let Some(event) = event {
                self.handle_event(event, control);
            }
        }
    }

    // Apply the new ControlFlow that has been selected by the user
    // Start any necessary timeouts etc
    fn apply_control_flow(&self, control_flow: root::ControlFlow) {
        let new_state = match control_flow {
            root::ControlFlow::Poll => {
                let cloned = self.clone();
                State::Poll {
                    request: backend::AnimationFrameRequest::new(move || cloned.poll()),
                }
            }
            root::ControlFlow::Wait => State::Wait {
                start: Instant::now(),
            },
            root::ControlFlow::WaitUntil(end) => {
                let start = Instant::now();

                let delay = if end <= start {
                    Duration::from_millis(0)
                } else {
                    end - start
                };

                let cloned = self.clone();

                State::WaitUntil {
                    start,
                    end,
                    timeout: backend::Timeout::new(
                        move || cloned.resume_time_reached(start, end),
                        delay,
                    ),
                }
            }
            root::ControlFlow::Exit => State::Exit,
        };

        match *self.0.runner.borrow_mut() {
            Some(ref mut runner) => {
                runner.state = new_state;
            }
            None => (),
        }
    }

    // Check if the event loop is currently closed
    fn is_closed(&self) -> bool {
        match *self.0.runner.borrow() {
            Some(ref runner) => runner.state.is_exit(),
            None => false, // If the event loop is None, it has not been intialised yet, so it cannot be closed
        }
    }

    // Get the current control flow state
    fn current_control_flow(&self) -> root::ControlFlow {
        match *self.0.runner.borrow() {
            Some(ref runner) => runner.state.control_flow(),
            None => root::ControlFlow::Poll,
        }
    }
}
