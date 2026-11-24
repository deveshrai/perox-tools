//! Which icon each command shows in menus, the toolbar and the palette.

use perox_icons::Icon;

pub fn for_command(id: &str) -> Icon {
    match id {
        "project.open_dialog" | "project.open" => Icon::FolderOpen,
        "project.new_dialog" | "project.new" => Icon::AppWindow,
        "form.new_window_dialog" => Icon::AppWindow,
        "form.new_dialog_dialog" | "form.new" => Icon::MessageSquareWarning,
        "file.save" => Icon::Save,
        "file.save_all" => Icon::SaveAll,
        "file.close" => Icon::X,
        "app.settings" => Icon::Settings,
        "app.quit" => Icon::LogOut,
        "view.quick_open" | "file.open" => Icon::FileSearch,
        "editor.goto_line_prompt" | "editor.goto_line" => Icon::ArrowDownToLine,
        "editor.references" => Icon::SearchCode,
        "editor.code_actions" => Icon::Lightbulb,
        "editor.rename_prompt" | "editor.rename" => Icon::TextCursorInput,
        "editor.format" => Icon::Paintbrush,
        "editor.toggle_format_on_save" => Icon::FileCheck,
        "view.command_palette" => Icon::Command,
        "view.reset_layout" => Icon::LayoutDashboard,
        "build.check" => Icon::CircleCheck,
        "build.build" => Icon::Hammer,
        "build.run" => Icon::Play,
        "build.test" => Icon::FlaskConical,
        "build.clippy" => Icon::ShieldCheck,
        "build.stop" => Icon::Square,
        "mcp.toggle" => Icon::Plug,
        "lsp.restart" => Icon::RotateCcw,
        "terminal.show" | "terminal.run" | "terminal.read" => Icon::Terminal,
        "view.source_control" | "git.status" => Icon::GitBranch,
        "git.diff" => Icon::FileDiff,
        "git.commit" => Icon::GitCommitHorizontal,
        "help.about" | "app.info" => Icon::Info,
        "designer.open_handler" => Icon::ArrowRight,
        "designer.add_widget" => Icon::Plus,
        "designer.go_to_handler" => Icon::ArrowRight,
        "designer.remove_widget" => Icon::Trash2,
        "designer.set_handler" => Icon::Zap,
        _ if id.starts_with("designer.") => Icon::PencilRuler,
        _ if id.starts_with("mcp.") => Icon::Settings2,
        _ => Icon::ListChecks,
    }
}

/// The Explorer's icon for a file, by extension.
pub fn for_file(path: &std::path::Path) -> Icon {
    match path.extension().and_then(|e| e.to_str()) {
        Some("rs") => Icon::FileCode,
        Some("toml" | "lock") => Icon::FileCog,
        Some("slint") => Icon::PencilRuler,
        Some("png" | "svg" | "jpg") => Icon::FileImage,
        Some("md" | "txt") => Icon::FileText,
        _ => Icon::File,
    }
}
