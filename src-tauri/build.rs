// Tauri 的编译期准备:生成图标、权限清单与配置的静态产物。
// 没有这个脚本,tauri::generate_context! 宏会找不到编译期数据。
fn main() {
    tauri_build::build()
}
