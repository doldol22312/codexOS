mod storage;
mod system;

pub(super) fn handle_disk_command() {
    storage::handle_disk_command();
}

pub(super) fn handle_fsinfo_command() {
    storage::handle_fsinfo_command();
}

pub(super) fn handle_fsformat_command() {
    storage::handle_fsformat_command();
}

pub(super) fn handle_fsls_command() {
    storage::handle_fsls_command();
}

pub(super) fn handle_fswrite_command<'a, I>(parts: I)
where
    I: Iterator<Item = &'a str>,
{
    storage::handle_fswrite_command(parts);
}

pub(super) fn handle_fsdelete_command<'a, I>(parts: I)
where
    I: Iterator<Item = &'a str>,
{
    storage::handle_fsdelete_command(parts);
}

pub(super) fn handle_fscat_command<'a, I>(parts: I)
where
    I: Iterator<Item = &'a str>,
{
    storage::handle_fscat_command(parts);
}

pub(super) fn handle_elfrun_command<'a, I>(parts: I)
where
    I: Iterator<Item = &'a str>,
{
    storage::handle_elfrun_command(parts);
}

pub(super) fn print_date() {
    system::print_date();
}

pub(super) fn print_time() {
    system::print_time();
}

pub(super) fn handle_rtc_command() {
    system::handle_rtc_command();
}

pub(super) fn handle_paging_command() {
    system::handle_paging_command();
}

pub(super) fn handle_mouse_command() {
    system::handle_mouse_command();
}

pub(super) fn handle_netinfo_command() {
    system::handle_netinfo_command();
}

pub(super) fn handle_discordcfg_command() {
    system::handle_discordcfg_command();
}

pub(super) fn handle_discorddiag_command() {
    system::handle_discorddiag_command();
}

pub(super) fn handle_memtest_command<'a, I>(parts: I)
where
    I: Iterator<Item = &'a str>,
{
    system::handle_memtest_command(parts);
}

pub(super) fn handle_hexdump_command<'a, I>(parts: I)
where
    I: Iterator<Item = &'a str>,
{
    system::handle_hexdump_command(parts);
}

pub(super) fn handle_color_command<'a, I>(parts: I)
where
    I: Iterator<Item = &'a str>,
{
    system::handle_color_command(parts);
}
