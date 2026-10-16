enum State {
    Ready,
    Busy,
    Done,
}

struct Job {
    state: State,
    value: i32,
}

fn next(state: State) -> State {
    match state {
        State::Ready => State::Busy,
        State::Busy => State::Done,
        State::Done => State::Ready,
    }
}

fn score(job: Job) -> i32 {
    let base = match job.state {
        State::Ready => 10,
        State::Busy => 20,
        State::Done => 30,
    };
    match job.state {
        State::Ready => {}
        State::Busy => {}
        State::Done => {}
    }
    base + job.value
}

fn main() {
    let state = next(State::Ready);
    let answer = score(Job {
        state: state,
        value: 7,
    });
    let valid = match answer {
        27 => true,
        _ => false,
    };
    match valid {
        true => println!("enum matches compiled correctly"),
        false => {}
    }
}
