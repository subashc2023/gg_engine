mod jobs_stress;
mod sandbox2d;

use gg_engine::prelude::*;

fn main() {
    // Use --stress flag to run the jobs stress test.
    if std::env::args().any(|a| a == "--stress") {
        run::<jobs_stress::JobsStress>();
    } else {
        run::<sandbox2d::Sandbox2D>();
    }
}
