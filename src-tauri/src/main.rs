// 必须是 windows 子系统：一旦写进开机启动，console 子系统会在登录时弹黑框。
#![windows_subsystem = "windows"]

fn main() {
    veery_lib::run()
}
