//! Toy models for the explorer's own tests (stage E1b, test plan section C). None of them uses the
//! brain. Each is small enough that its state count can be read off the definition.

use super::dst::Model;

/// Toy-A: two actors read a shared counter and then write back what they read plus one, so the final
/// counter must be 2. `pads` no-op events, each consuming one of them, lengthen a trace and nothing else.
#[derive(Clone, Debug)]
pub(crate) struct LostUpdate {
    pads: u32,
    counter: u32,
    actors: [Actor; 2],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Actor {
    Idle,
    Read(u32),
    Done,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum LostEvent {
    Pad,
    Read(usize),
    Write(usize),
}

impl LostUpdate {
    pub(crate) fn new(pads: u32) -> Self {
        Self { pads, counter: 0, actors: [Actor::Idle; 2] }
    }
}

impl Model for LostUpdate {
    type Event = LostEvent;
    type Key = (u32, u32, [Actor; 2]);

    fn key(&self) -> Self::Key {
        (self.pads, self.counter, self.actors)
    }

    fn enabled(&self) -> Vec<LostEvent> {
        let mut events = Vec::new();
        if self.pads > 0 {
            events.push(LostEvent::Pad);
        }
        for (actor, state) in self.actors.iter().enumerate() {
            match state {
                Actor::Idle => events.push(LostEvent::Read(actor)),
                Actor::Read(_) => events.push(LostEvent::Write(actor)),
                Actor::Done => {}
            }
        }
        events
    }

    /// Pads are one class, the actors' events another (the weighted walk's test).
    fn class(&self, event: &LostEvent) -> usize {
        usize::from(*event != LostEvent::Pad)
    }

    fn apply(&mut self, event: &LostEvent) {
        match *event {
            LostEvent::Pad => self.pads -= 1,
            LostEvent::Read(actor) => self.actors[actor] = Actor::Read(self.counter),
            LostEvent::Write(actor) => {
                if let Actor::Read(value) = self.actors[actor] {
                    self.counter = value + 1;
                }
                self.actors[actor] = Actor::Done;
            }
        }
    }

    fn safety(&self) -> Result<(), String> {
        if self.actors.iter().all(|actor| *actor == Actor::Done) && self.counter != 2 {
            return Err(format!("lost update: counter is {} after both writes, want 2", self.counter));
        }
        Ok(())
    }

    fn progress(&self) -> Result<(), String> {
        Ok(())
    }

    fn witnesses(&self) -> Vec<&'static str> {
        Vec::new()
    }
}

/// Toy-B: two actors take locks 0 and 1 in opposite orders. Each can hold its first lock while it
/// waits for the other's, so one schedule deadlocks (P4).
#[derive(Clone, Debug)]
pub(crate) struct LockOrder {
    pc: [usize; 2],
    owner: [Option<usize>; 2],
}

#[derive(Clone, Copy, Debug)]
enum Op {
    Lock(usize),
    Unlock(usize),
}

const PROGRAM: [[Op; 4]; 2] = [[Op::Lock(0), Op::Lock(1), Op::Unlock(1), Op::Unlock(0)], [Op::Lock(1), Op::Lock(0), Op::Unlock(0), Op::Unlock(1)]];

impl LockOrder {
    pub(crate) fn new() -> Self {
        Self { pc: [0; 2], owner: [None; 2] }
    }
}

impl Model for LockOrder {
    /// The actor that steps.
    type Event = usize;
    type Key = ([usize; 2], [Option<usize>; 2]);

    fn key(&self) -> Self::Key {
        (self.pc, self.owner)
    }

    fn enabled(&self) -> Vec<usize> {
        (0..2)
            .filter(|&actor| match PROGRAM[actor].get(self.pc[actor]) {
                Some(Op::Lock(lock)) => self.owner[*lock].is_none(),
                Some(Op::Unlock(_)) => true,
                None => false,
            })
            .collect()
    }

    fn apply(&mut self, actor: &usize) {
        let actor = *actor;
        match PROGRAM[actor][self.pc[actor]] {
            Op::Lock(lock) => self.owner[lock] = Some(actor),
            Op::Unlock(lock) => self.owner[lock] = None,
        }
        self.pc[actor] += 1;
    }

    fn safety(&self) -> Result<(), String> {
        Ok(())
    }

    fn progress(&self) -> Result<(), String> {
        if self.pc == [PROGRAM[0].len(); 2] {
            return Ok(());
        }
        Err(format!("deadlock: actors at {:?}, lock owners {:?}", self.pc, self.owner))
    }

    fn witnesses(&self) -> Vec<&'static str> {
        Vec::new()
    }
}

/// Toy-F: the protocol state (`flag`) repeats while the diagnostic counter `ticks`, which the key
/// leaves out, changes on every event.
#[derive(Clone, Debug)]
pub(crate) struct Flip {
    flag: bool,
    ticks: u32,
}

impl Flip {
    pub(crate) fn new() -> Self {
        Self { flag: false, ticks: 0 }
    }
}

impl Model for Flip {
    type Event = ();
    type Key = bool;

    fn key(&self) -> bool {
        self.flag
    }

    fn enabled(&self) -> Vec<()> {
        vec![()]
    }

    fn apply(&mut self, _: &()) {
        self.flag = !self.flag;
        self.ticks += 1;
    }

    fn safety(&self) -> Result<(), String> {
        Ok(())
    }

    fn progress(&self) -> Result<(), String> {
        Ok(())
    }

    fn witnesses(&self) -> Vec<&'static str> {
        Vec::new()
    }
}

/// Toy-C: one self-loop.
pub(crate) const SELF_LOOP: &[(u8, u8)] = &[(0, 0)];
/// Toy-D: a diamond; two paths meet at node 3.
pub(crate) const DIAMOND: &[(u8, u8)] = &[(0, 1), (0, 2), (1, 3), (2, 3)];
/// Toy-E: six states and seven edges, with two reconvergences.
pub(crate) const DAG: &[(u8, u8)] = &[(0, 1), (0, 2), (1, 3), (2, 3), (2, 4), (3, 5), (4, 5)];
/// Node 2 is entered first through (0, 2), and again through (0, 1) and (1, 2); (1, 2) is forbidden.
pub(crate) const SHORTCUT: &[(u8, u8)] = &[(0, 2), (0, 1), (1, 2)];

/// Toys C, D and E and the E1-61 toy: a graph given by its edge list. The enabled events are the
/// successors of the current node, in edge-list order. A step along a forbidden edge fails the
/// transition check, which reads the edge, not the state, so it is not part of the key.
#[derive(Clone, Debug)]
pub(crate) struct Graph {
    edges: &'static [(u8, u8)],
    forbidden: &'static [(u8, u8)],
    node: u8,
}

impl Graph {
    pub(crate) fn new(edges: &'static [(u8, u8)], forbidden: &'static [(u8, u8)]) -> Self {
        Self { edges, forbidden, node: 0 }
    }
}

impl Model for Graph {
    type Event = u8;
    type Key = u8;

    fn key(&self) -> u8 {
        self.node
    }

    fn enabled(&self) -> Vec<u8> {
        self.edges.iter().filter(|(from, _)| *from == self.node).map(|&(_, to)| to).collect()
    }

    fn apply(&mut self, to: &u8) {
        self.node = *to;
    }

    fn safety(&self) -> Result<(), String> {
        Ok(())
    }

    fn transition(&self, to: &u8, _after: &Self) -> Result<(), String> {
        if self.forbidden.contains(&(self.node, *to)) {
            return Err(format!("edge {} -> {} is forbidden", self.node, to));
        }
        Ok(())
    }

    fn progress(&self) -> Result<(), String> {
        Ok(())
    }

    fn witnesses(&self) -> Vec<&'static str> {
        if self.enabled().is_empty() { vec!["terminal"] } else { Vec::new() }
    }
}
