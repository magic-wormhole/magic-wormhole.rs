#[derive(Debug)]
enum BossEvent {
    B1,
    B2,
}
#[derive(Debug)]
enum KeyEvent {
    K1,
    K2,
}
#[derive(Debug)]
enum MachineEvent {
    Boss(BossEvent),
    Key(KeyEvent),
}

//use std::collections::Vec;

#[derive(Debug)]
struct MyVec {
    events: Vec<MachineEvent>,
}
impl MyVec {
    fn new() -> MyVec {
        MyVec { events: vec![] }
    }
}

impl MyVec {
    //fn add<T>(&mut self, item: T) where T: Into<MachineEvent> {
    fn push<T>(&mut self, item: T) where MachineEvent: std::convert::From<T> {
        self.events.push(MachineEvent::from(item));
    }
}

macro_rules! myvec {
    ( $( $x:expr ),* ) => {
        {
            let mut temp_vec = MyVec::new();
            $(
                temp_vec.push($x);
            )*
            temp_vec
        }
    };
}

impl From<BossEvent> for MachineEvent {
    fn from(r: BossEvent) -> Self {
        MachineEvent::Boss(r)
    }
}

impl From<KeyEvent> for MachineEvent {
    fn from(r: KeyEvent) -> Self {
        MachineEvent::Key(r)
    }
}

fn dispatch_boss(e: BossEvent) -> MyVec {
    /*let mut v = MyVec::new();
    v.push(KeyEvent::K1);
    v*/
    myvec![KeyEvent::K1]
}

fn dispatch_key(e: KeyEvent) -> MyVec {
    let mut v = MyVec::new();
    v.push(BossEvent::B2);
    v
}

fn dispatch(me: MachineEvent) -> MyVec {
    match me {
        MachineEvent::Boss(e) => dispatch_boss(e),
        MachineEvent::Key(e) => dispatch_key(e)
    }
}

trait T1 {
    fn foo(&self) -> u8 {
        1
    }
}

impl T1 for BossEvent { }
impl T1 for KeyEvent { }
use std::fmt::Debug;

fn main() {
    let x = 1;
    let (a, b) = match x {
        1 => (2, 3),
        2 => (3, 5),
        _ => (4, 9),
    };
    println!("Hello, world! {} {}", a, b);
    let mut v = MyVec { events: vec![] };
    v.push(BossEvent::B1);
    v.push(KeyEvent::K1);
    println!("v: {:?}", v);
    println!("dv: {:?}", dispatch(MachineEvent::Boss(BossEvent::B1)));
    let mut v2: Vec<&T1> = vec![];
    {
        v2.push(&BossEvent::B1);
    }
    v2.push(&KeyEvent::K1);
    //println!("v2: {:?}", v2);
    for e in v2 {
        println!("e: {:?}", e);
    }
}
