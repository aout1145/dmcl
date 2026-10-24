//! 「关于」对话框。

use relm4::adw;
use relm4::adw::prelude::*;
use relm4::gtk;

/// 在主窗口上打开「关于」。
pub(super) fn show(parent: &adw::ApplicationWindow) {
    let about = adw::AboutDialog::new();
    about.set_application_name("DMCL");
    about.set_application_icon("io.github.aout1145.DMCL");
    about.set_version(env!("CARGO_PKG_VERSION"));
    about.set_developer_name("a_out_");
    about.set_comments("非官方 Minecraft 启动器");
    about.set_license_type(gtk::License::Gpl30);
    about.present(Some(parent));
}
