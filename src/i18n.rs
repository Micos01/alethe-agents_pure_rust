//! Strings taken from `src/lib/i18n/messages/{pt-BR,en}.ts` of the web build.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    PtBr,
    En,
}

impl Lang {
    pub fn from_code(code: &str) -> Self {
        if code.eq_ignore_ascii_case("en") {
            Lang::En
        } else {
            Lang::PtBr
        }
    }
}

macro_rules! messages {
    ($($key:literal => $pt:literal, $en:literal;)*) => {
        pub fn tr(lang: Lang, key: &str) -> &'static str {
            match (key, lang) {
                $(($key, Lang::PtBr) => $pt, ($key, Lang::En) => $en,)*
                _ => "?",
            }
        }
    };
}

messages! {
    "greeting.morning" => "Bom dia", "Good morning";
    "greeting.afternoon" => "Boa tarde", "Good afternoon";
    "greeting.evening" => "Boa noite", "Good evening";
    "greeting.fallbackName" => "dev", "dev";
    "home.quickTerminalTitle" => "alethe@workspace:~", "alethe@workspace:~";
    "home.quickPromptPlaceholder" => "O que você quer construir?", "What do you want to build?";
    "home.quickAgentShort" => "Agente", "Agent";
    "home.quickPath" => "Pasta", "Folder";
    "home.quickFolderPlaceholder" => "Escolha uma pasta", "Choose a folder";
    "home.quickMode" => "Modo", "Mode";
    "home.quickRestricted" => "Normal", "Normal";
    "home.quickUnrestricted" => "YOLO", "YOLO";
    "home.quickSend" => "Iniciar agente", "Start agent";
    "home.newTerminal" => "novo terminal", "new terminal";
    "home.newProject" => "novo projeto", "new project";
    "home.newGroup" => "novo grupo", "new group";
    "home.recentProjects" => "projetos recentes", "recent projects";
    "home.viewAll" => "Ver todos", "View all";
    "home.startSomething" => "comece algo", "start something";
    "home.projectsEmptyTitle" => "Nenhum projeto ainda", "No projects yet";
    "home.projectsEmptyDesc" => "Crie seu primeiro projeto e comece a organizar os terminais agora.", "Create your first project and start organizing your terminals.";
    "home.projectsEmptyAction" => "Criar projeto", "Create project";
    "home.terminalsOne" => "terminal", "terminal";
    "home.terminalsMany" => "terminais", "terminals";
    "time.now" => "agora", "now";
    "time.days" => "dias", "days";
    "time.yesterday" => "ontem", "yesterday";
    "sidebar.home" => "Home", "Home";
    "sidebar.search" => "Buscar", "Search";
    "sidebar.projects" => "Projetos", "Projects";
    "sidebar.ungrouped" => "Sem grupo", "Ungrouped";
    "sidebar.workspace" => "Workspace", "Workspace";
    "sidebar.organization" => "Organização", "Organization";
    "sidebar.newTerminal" => "Novo terminal", "New terminal";
    "sidebar.newProject" => "Novo projeto", "New project";
    "sidebar.newProjectFromFolder" => "Novo projeto a partir de pasta", "New project from folder";
    "sidebar.newGroup" => "Novo grupo", "New group";
    "sidebar.rename" => "Renomear", "Rename";
    "sidebar.delete" => "Excluir", "Delete";
    "sidebar.close" => "Fechar", "Close";
    "sidebar.moveToGroup" => "Mover para grupo", "Move to group";
    "sidebar.removeFromGroup" => "Remover do grupo", "Remove from group";
    "sidebar.openFolder" => "Abrir pasta", "Open folder";
    "sidebar.restart" => "Reiniciar", "Restart";
    "sidebar.color" => "Cor", "Color";
    "sidebar.local" => "conta local", "local account";
    "sidebar.noResults" => "Nada encontrado", "Nothing found";
    "pane.newTab" => "Nova aba", "New tab";
    "pane.maximize" => "Maximizar", "Maximize";
    "pane.restore" => "Restaurar", "Restore";
    "pane.close" => "Fechar terminal", "Close terminal";
    "pane.exited" => "Processo encerrado", "Process exited";
    "pane.restart" => "reiniciar", "restart";
    "pane.unsupported" => "Este tipo de painel ainda não está disponível na versão nativa.", "This pane type is not available in the native build yet.";
    "pane.copy" => "Copiar", "Copy";
    "pane.paste" => "Colar", "Paste";
    "pane.clear" => "Limpar", "Clear";
    "workspace.empty" => "Nenhum terminal neste projeto", "No terminals in this project";
    "workspace.emptyDesc" => "Abra um agente ou um shell para começar.", "Open an agent or a shell to start.";
    "layout.auto" => "Automático", "Auto";
    "layout.grid" => "Grade", "Grid";
    "layout.spotlight" => "Destaque", "Spotlight";
    "layout.sidebar" => "Lateral", "Sidebar";
    "todo.tab" => "Todo", "Todo";
    "todo.title" => "Tarefas", "Tasks";
    "todo.open" => "em aberto", "open";
    "todo.addPlaceholder" => "Adicionar uma tarefa…", "Add a task…";
    "todo.tags" => "Tags: docs, fix", "Tags: docs, fix";
    "todo.noProject" => "Nenhum projeto", "No project";
    "todo.emptyTitle" => "Nada na sua lista", "Nothing on your list";
    "todo.emptyDescription" => "Adicione uma tarefa e mantenha ela visível enquanto trabalha.", "Add a task and keep it visible while you work.";
    "settings.title" => "Preferências", "Preferences";
    "settings.appearance" => "Aparência", "Appearance";
    "settings.theme" => "Tema da interface", "Interface theme";
    "settings.terminalTheme" => "Tema do terminal", "Terminal theme";
    "settings.sameAsUi" => "Igual à interface", "Same as interface";
    "settings.language" => "Idioma", "Language";
    "settings.zoom" => "Zoom da interface", "Interface zoom";
    "settings.fontSize" => "Fonte do terminal", "Terminal font size";
    "settings.profile" => "Perfil", "Profile";
    "settings.displayName" => "Nome exibido", "Display name";
    "settings.shell" => "Shell padrão", "Default shell";
    "settings.agents" => "Agentes", "Agents";
    "settings.data" => "Dados", "Data";
    "settings.dataFile" => "Arquivo de projetos", "Projects file";
    "settings.reimport" => "Reimportar do Alethe (Tauri)", "Re-import from Alethe (Tauri)";
    "settings.reimportHint" => "Substitui os projetos da versão nativa pela cópia atual do app Tauri.", "Replaces the native projects with the Tauri app's current copy.";
    "settings.done" => "Concluído", "Done";
    "settings.notFound" => "não encontrado", "not found";
    "dialog.newProject" => "Novo projeto", "New project";
    "dialog.newGroup" => "Novo grupo", "New group";
    "dialog.rename" => "Renomear", "Rename";
    "dialog.name" => "Nome", "Name";
    "dialog.folder" => "Pasta", "Folder";
    "dialog.browse" => "Procurar…", "Browse…";
    "dialog.create" => "Criar", "Create";
    "dialog.save" => "Salvar", "Save";
    "dialog.cancel" => "Cancelar", "Cancel";
    "dialog.deleteProject" => "Excluir projeto?", "Delete project?";
    "dialog.deleteProjectBody" => "Os terminais deste projeto serão encerrados. Os arquivos em disco não são tocados.", "This project's terminals will be closed. Files on disk are not touched.";
    "status.working" => "trabalhando", "working";
    "status.waiting" => "aguardando", "waiting";
    "status.stopped" => "parado", "stopped";
    "status.offline" => "encerrado", "exited";
    "toast.imported" => "Projetos importados do Alethe (Tauri)", "Projects imported from Alethe (Tauri)";
    "toast.saveFailed" => "Falha ao salvar projetos", "Failed to save projects";
    "menu.preferences" => "Preferências", "Preferences";
    "menu.toggleSidebar" => "Alternar barra lateral", "Toggle sidebar";
    "menu.toggleRight" => "Alternar painel de tarefas", "Toggle tasks panel";
    "menu.quit" => "Sair", "Quit";
    "menu.about" => "Sobre", "About";
}
