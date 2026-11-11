use std::{cell::RefCell, collections::VecDeque, rc::Rc};

use gpui_kit::{App, Entity, WeakEntity};

use crate::{app::file_tree::Request, infra, ui::Viewer};

#[derive(Default)]
struct Work {
    queue: VecDeque<Request>,
    running: usize,
}

pub(crate) fn connect_file_tree(view: &Entity<Viewer>, cx: &mut App) {
    let work = Rc::new(RefCell::new(Work::default()));
    cx.subscribe(view, move |view, request: &Request, cx| {
        work.borrow_mut().queue.push_back(request.clone());
        dispatch(&work, &view.downgrade(), cx);
    })
    .detach();
}

fn dispatch(work: &Rc<RefCell<Work>>, view: &WeakEntity<Viewer>, cx: &mut App) {
    loop {
        let request = {
            let mut work = work.borrow_mut();
            if work.running >= 4 {
                return;
            }
            let Some(request) = work.queue.pop_front() else {
                return;
            };
            work.running += 1;
            request
        };
        let result = cx
            .background_executor()
            .spawn(async move { infra::file_tree::execute(request) });
        let view = view.clone();
        let work = work.clone();
        cx.spawn(async move |cx| {
            let completion = result.await;
            let _ = view.update(cx, |view, cx| view.complete_tree(completion, cx));
            work.borrow_mut().running -= 1;
            cx.update(|cx| dispatch(&work, &view, cx));
        })
        .detach();
    }
}
