use std::error::Error;
use std::io::{self, Write};

use operation_logger::{
    AppWindow, create_operation_log_directory, list_running_applications, start_input_logging,
};

fn main() -> Result<(), Box<dyn Error>> {
    let apps = list_running_applications()?;

    if apps.is_empty() {
        println!("No applications found.");
        return Ok(());
    }

    print_applications(&apps);

    let selected = select_application(&apps)?;
    let log_dir = create_operation_log_directory(selected)?;

    println!("Created log directory: {}", log_dir.display());
    let session = start_input_logging(selected, &log_dir)?;

    println!("Input logging started. Press Enter to stop.");
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    session.stop()?;

    Ok(())
}

fn print_applications(apps: &[AppWindow]) {
    for (index, app) in apps.iter().enumerate() {
        println!(
            "{}: {} | pid: {} | process: {}",
            index + 1,
            app.title,
            app.process_id,
            app.process_name.as_deref().unwrap_or("(unknown)")
        );
    }
}

fn select_application(apps: &[AppWindow]) -> io::Result<&AppWindow> {
    loop {
        print!("Select application number: ");
        io::stdout().flush()?;

        let mut input = String::new();
        if io::stdin().read_line(&mut input)? == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "no application number was provided",
            ));
        }

        match input.trim().parse::<usize>() {
            Ok(number) if (1..=apps.len()).contains(&number) => return Ok(&apps[number - 1]),
            _ => println!("Please enter a number between 1 and {}.", apps.len()),
        }
    }
}
