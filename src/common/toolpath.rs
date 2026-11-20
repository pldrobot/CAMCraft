/// One straight piece of the toolpath, for the preview.
#[derive(Clone, Copy, Debug)]
pub struct Move {
    pub from: [f64; 3],
    pub to: [f64; 3],
    /// True while the end effector is on.
    pub work: bool,
}
