use super::super::*;

pub(super) fn handle_disk_command() {
    let Some(info) = ata::info() else {
        shell_println!("ata disk: unavailable");
        return;
    };

    let model_end = info
        .model
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(info.model.len());
    let model = core::str::from_utf8(&info.model[..model_end]).unwrap_or("unknown");
    let mib = (info.sectors as u64 * info.sector_size as u64) / (1024 * 1024);

    shell_println!(
        "ata disk: {}",
        if info.present { "present" } else { "missing" }
    );
    shell_println!("model: {}", model);
    shell_println!(
        "capacity: {} sectors ({} bytes, {} MiB)",
        info.sectors,
        info.sectors as u64 * info.sector_size as u64,
        mib
    );
}

pub(super) fn handle_fsinfo_command() {
    let info = fs::info();
    shell_println!(
        "filesystem: {}",
        if info.mounted { "mounted" } else { "unmounted" }
    );
    shell_println!(
        "disk: {}",
        if info.disk_present {
            "present"
        } else {
            "missing"
        }
    );
    if !info.mounted {
        shell_println!("hint: run `fsformat` once to initialize");
        return;
    }

    shell_println!("total sectors: {}", info.total_sectors);
    shell_println!("directory sectors: {}", info.directory_sectors);
    shell_println!("next free lba: {}", info.next_free_lba);
    shell_println!("file count: {}", info.file_count);
    shell_println!("free sectors: {}", info.free_sectors);
}

pub(super) fn handle_fsformat_command() {
    match fs::format() {
        Ok(()) => {
            shell_println!("filesystem formatted and mounted");
            handle_fsinfo_command();
        }
        Err(error) => shell_println!("fsformat failed: {}", error.as_str()),
    }
}

pub(super) fn handle_fsls_command() {
    let mut files = [fs::FileInfo::empty(); 64];
    let listed = match fs::list(&mut files) {
        Ok(value) => value,
        Err(error) => {
            shell_println!("fsls failed: {}", error.as_str());
            return;
        }
    };

    if listed == 0 {
        shell_println!("filesystem is empty");
        return;
    }

    shell_println!("files ({}):", listed);
    for file in files.iter().take(listed) {
        shell_println!("  {} ({} bytes)", file.name_str(), file.size_bytes);
    }
}

pub(super) fn handle_fswrite_command<'a, I>(mut parts: I)
where
    I: Iterator<Item = &'a str>,
{
    let Some(name) = parts.next() else {
        shell_println!("usage: fswrite <name> <text>");
        return;
    };

    let mut data = [0u8; 4096];
    let mut length = 0usize;
    let mut first = true;

    for part in parts {
        if !first {
            if length >= data.len() {
                shell_println!("fswrite failed: text too long (max {} bytes)", data.len());
                return;
            }
            data[length] = b' ';
            length += 1;
        }

        for byte in part.bytes() {
            if length >= data.len() {
                shell_println!("fswrite failed: text too long (max {} bytes)", data.len());
                return;
            }
            data[length] = byte;
            length += 1;
        }
        first = false;
    }

    match fs::write_file(name, &data[..length]) {
        Ok(()) => shell_println!("wrote {} bytes to {}", length, name),
        Err(error) => shell_println!("fswrite failed: {}", error.as_str()),
    }
}

pub(super) fn handle_fsdelete_command<'a, I>(mut parts: I)
where
    I: Iterator<Item = &'a str>,
{
    let Some(name) = parts.next() else {
        shell_println!("usage: fsdelete <name>");
        return;
    };

    match fs::delete_file(name) {
        Ok(()) => shell_println!("deleted {}", name),
        Err(error) => shell_println!("fsdelete failed: {}", error.as_str()),
    }
}

pub(super) fn handle_fscat_command<'a, I>(mut parts: I)
where
    I: Iterator<Item = &'a str>,
{
    let Some(name) = parts.next() else {
        shell_println!("usage: fscat <name>");
        return;
    };

    let mut buffer = [0u8; 4096];
    match fs::read_file(name, &mut buffer) {
        Ok(result) => {
            if result.total_size == 0 {
                shell_println!("{} is empty", name);
                return;
            }

            let text = core::str::from_utf8(&buffer[..result.copied_size]).unwrap_or("<binary>");
            shell_println!("{} ({} bytes):", name, result.total_size);
            shell_println!("{}", text);
        }
        Err(error) => shell_println!("fscat failed: {}", error.as_str()),
    }
}

pub(super) fn handle_elfrun_command<'a, I>(mut parts: I)
where
    I: Iterator<Item = &'a str>,
{
    let Some(path) = parts.next() else {
        shell_println!("usage: elfrun <name>");
        return;
    };

    if parts.next().is_some() {
        shell_println!("usage: elfrun <name>");
        return;
    }

    match elf::spawn_from_fs(path) {
        Ok(task_id) => {
            shell_println!("elfrun: started {} as task {}", path, task_id);
            while task::is_task_alive(task_id) {
                task::yield_now();
            }
        }
        Err(error) => {
            shell_println!("elfrun failed: {}", error.as_str());
        }
    }
}
