//! `app.run(title)`: the window and the main loop. Samples input, runs the app's frame, draws.
use crate::{
    assets::{Assets, Material, Mesh},
    cams,
    gpu::{HEIGHT, WIDTH},
    input::Input,
    renderer::Renderer,
};
use core::ffi::CStr;
use ecs::App;
use platform::input::{Keys, View as PlatformView};

#[cfg(feature = "mouse")]
use crate::mouse::{Mouse, MouseCapture};
#[cfg(feature = "post")]
use crate::post::PostProcess;

/// `app.run(title)`: open a window and drive the app until it closes.
/// Adds the `Assets<Mesh>`, `Assets<Material>` and `Input` resources first (plus `FixedInput` with `fixed_input`,
/// `PostProcess` with `post`), so startup systems can use them.
pub trait RunExt {
    fn run(self, title: &CStr);
}
impl RunExt for App {
    fn run(self, title: &CStr) {
        unsafe { run(self, title) }
    }
}

unsafe fn run(mut app: App, title: &CStr) {
    app.world.insert_resource(Assets::<Mesh>::default());
    app.world.insert_resource(Assets::<Material>::default());
    app.world.insert_resource(Input::default());
    #[cfg(feature = "fixed_input")]
    crate::fixed_input::init(&mut app);
    #[cfg(feature = "mouse")]
    if app.world.resource::<MouseCapture>().is_none() {
        app.world.insert_resource(MouseCapture(false));
    }
    cams::ensure_camera(&mut app);
    #[cfg(feature = "post")]
    app.world.insert_resource(PostProcess::default());

    let w = platform::open(WIDTH, HEIGHT, title);
    let mut r = Renderer::new(|instance| platform::surface(instance, &w));

    let mut resized = false;
    let mut shown = false;
    let mut keys = Keys::default();
    #[cfg(feature = "mouse")]
    let mut was_down = false;
    let mut last = platform::now();
    let start = last;

    loop {
        if !platform::poll(&w, &mut resized, &mut keys) {
            break;
        }
        let now = platform::now();
        let dt = (now - last) as f32;
        last = now;
        let changes = keys.take_changes();

        #[cfg(feature = "mouse")]
        let mouse = {
            let e = r.extent();
            let captured = app.world.resource::<MouseCapture>().unwrap().0;
            let delta = if captured {
                platform::mouse_delta(&w, (e.width / 2) as i32, (e.height / 2) as i32)
            } else {
                [0.0; 2]
            };
            let pointer = if captured {
                None
            } else {
                platform::pointer(&w)
            };
            let down = pointer.is_some_and(|p| p.1);
            let m = Mouse {
                delta,
                cursor: pointer.map(|p| p.0),
                #[cfg(feature = "cam2d")]
                size: [e.width as f32, e.height as f32],
                down,
                clicked: down && !was_down,
            };
            was_down = down;
            m
        };

        *app.world.resource_mut::<Input>().unwrap() = Input {
            view: PlatformView::frame(keys.held(), &changes),
            #[cfg(feature = "mouse")]
            mouse,
        };
        #[cfg(feature = "fixed_input")]
        {
            #[cfg(feature = "mouse")]
            let delta = mouse.delta;
            #[cfg(not(feature = "mouse"))]
            let delta = [0.0; 2];
            app.world
                .resource_mut::<crate::fixed_input::FixedInput>()
                .unwrap()
                .feed(&changes, delta);
        }
        app.run_frame(dt);

        if r.draw(&app, &mut resized, (now - start) as f32) && !shown {
            platform::show(&w);
            shown = true;
        }
    }

    // no teardown, the OS reclaims everything on exit
}
