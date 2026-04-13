use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    operation_logger::gui::run()
}
