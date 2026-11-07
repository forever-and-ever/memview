use crossterm::event::KeyCode;
use tui_eternal::{Binding, Command, Section, Shortcut};

macro_rules! tab_catalog {
    ($($variant:ident => {
        title: $title:literal,
        token: $token:literal,
        scans: $scans:literal,
        navigation: $navigation:expr,
        bindings: $bindings:expr
    }),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        #[repr(u8)]
        pub enum Tab {$($variant),+}

        impl Tab {
            pub const ALL: [Self; tab_catalog!(@count $($variant)+)] = [$(Self::$variant),+];

            #[must_use]
            pub fn next(self) -> Self {
                Self::ALL[(self as usize + 1) % Self::ALL.len()]
            }

            #[must_use]
            pub fn previous(self) -> Self {
                Self::ALL[(self as usize + Self::ALL.len() - 1) % Self::ALL.len()]
            }

            #[must_use]
            pub fn title(self) -> &'static str {
                match self {$(Self::$variant => $title),+}
            }

            #[must_use]
            pub fn token(self) -> &'static str {
                match self {$(Self::$variant => $token),+}
            }

            #[must_use]
            pub fn from_token(token: &str) -> Option<Self> {
                match token {$($token => Some(Self::$variant),)+ _ => None}
            }

            #[must_use]
            pub fn drives_process_scans(self) -> bool {
                match self {$(Self::$variant => $scans),+}
            }

            #[must_use]
            pub fn bindings(self) -> &'static [Binding<Action>] {
                match self {$(Self::$variant => $bindings),+}
            }

            #[must_use]
            pub fn navigation(self) -> &'static [Binding<Action>] {
                match self {$(Self::$variant => $navigation),+}
            }
        }
    };
    (@count $head:ident $($tail:ident)*) => {1usize $(+ tab_catalog!(@one $tail))*};
    (@one $variant:ident) => {1usize};
}

tab_catalog! {
    Overview => {
        title: "Overview",
        token: "overview",
        scans: false,
        navigation: &[],
        bindings: OVERVIEW_BINDINGS
    },
    Processes => {
        title: "Processes",
        token: "processes",
        scans: true,
        navigation: TREE_NAVIGATION,
        bindings: PROCESS_ACTIONS
    },
    Tmpfs => {
        title: "Tmpfs",
        token: "tmpfs",
        scans: false,
        navigation: TREE_NAVIGATION,
        bindings: TMPFS_ACTIONS
    },
    Shared => {
        title: "Shared",
        token: "shared",
        scans: false,
        navigation: FLAT_NAVIGATION,
        bindings: SHARED_ACTIONS
    },
}

impl Tab {
    /// The sections in force on this tab, most specific first.
    pub fn sections(self) -> [Section<Action>; 3] {
        [
            Section {
                title: self.title(),
                bindings: self.bindings(),
            },
            Section {
                title: "Navigation",
                bindings: self.navigation(),
            },
            APPLICATION,
        ]
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Action {
    Quit,
    ShowHelp,
    OpenSearch,
    ClearSearch,
    NextTab,
    PreviousTab,
    SelectTab(Tab),
    CycleMetric,
    CycleScope,
    Refresh,
    Kill,
    Move(isize),
    PageUp,
    PageDown,
    Fold(FoldAction),
    FirstRow,
    LastRow,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FoldAction {
    Collapse,
    Expand,
    Toggle,
    CollapseRecursive,
    ExpandRecursive,
    ToggleRecursive,
    CollapseAll,
    ExpandAll,
    RevealLevel,
    HideLevel,
}

const APPLICATION: Section<Action> = Section {
    title: "Application",
    bindings: &[
        Binding::new(
            "1-4",
            "go to the pane of that number",
            &[
                Command(Shortcut::Char('1'), Action::SelectTab(Tab::Overview)),
                Command(Shortcut::Char('2'), Action::SelectTab(Tab::Processes)),
                Command(Shortcut::Char('3'), Action::SelectTab(Tab::Tmpfs)),
                Command(Shortcut::Char('4'), Action::SelectTab(Tab::Shared)),
            ],
        ),
        Binding::new(
            "Tab/Shift-Tab",
            "go to the next or previous pane",
            &[
                Command(Shortcut::Code(KeyCode::Tab), Action::NextTab),
                Command(Shortcut::Code(KeyCode::BackTab), Action::PreviousTab),
            ],
        ),
        Binding::new(
            "/",
            "filter the current ledger's rows by regexp; an empty search clears",
            &[Command(Shortcut::Char('/'), Action::OpenSearch)],
        )
        .hint("/", "search"),
        Binding::new(
            "f",
            "clear the active regexp filter",
            &[Command(Shortcut::Char('f'), Action::ClearSearch)],
        ),
        Binding::new(
            "?",
            "show this help",
            &[Command(Shortcut::Char('?'), Action::ShowHelp)],
        )
        .hint("?", "help"),
        Binding::new(
            "q Ctrl-C",
            "quit",
            &[
                Command(Shortcut::Char('q'), Action::Quit),
                Command(Shortcut::Ctrl('c'), Action::Quit),
            ],
        )
        .hint("q", "quit"),
    ],
};

const OVERVIEW_BINDINGS: &[Binding<Action>] = &[
    Binding::new(
        "r",
        "refresh kernel counters and SysV shm",
        &[Command(Shortcut::Char('r'), Action::Refresh)],
    )
    .hint("r", "refresh"),
    Binding::new(
        "s",
        "cycle memory lens",
        &[Command(Shortcut::Char('s'), Action::CycleMetric)],
    )
    .hint("s", "lens"),
];

const MOVE: Binding<Action> = Binding::new(
    "j/k ↓/↑",
    "move the selection down or up",
    &[
        Command(Shortcut::Char('j'), Action::Move(1)),
        Command(Shortcut::Char('k'), Action::Move(-1)),
        Command(Shortcut::Code(KeyCode::Down), Action::Move(1)),
        Command(Shortcut::Code(KeyCode::Up), Action::Move(-1)),
    ],
)
.hint("j/k/Pg/wheel", "move");

const EDGE: Binding<Action> = Binding::new(
    "gg/G",
    "jump to the first or last row",
    &[
        Command(Shortcut::Seq('g', 'g'), Action::FirstRow),
        Command(Shortcut::Char('G'), Action::LastRow),
    ],
)
.hint("gg/G", "edge");

const PAGE: Binding<Action> = Binding::new(
    "PgUp/PgDn",
    "move the selection by one visible pane, up or down",
    &[
        Command(Shortcut::Code(KeyCode::PageUp), Action::PageUp),
        Command(Shortcut::Code(KeyCode::PageDown), Action::PageDown),
    ],
);

const WHEEL: Binding<Action> = Binding::new("wheel", "move the selection one row per detent", &[]);

const FLAT_NAVIGATION: &[Binding<Action>] = &[MOVE, EDGE, PAGE, WHEEL];

const TREE_NAVIGATION: &[Binding<Action>] = &[
    MOVE,
    EDGE,
    PAGE,
    WHEEL,
    Binding::new(
        "h/l ←/→",
        "collapse or expand the selected subtree",
        &[
            Command(Shortcut::Char('h'), Action::Fold(FoldAction::Collapse)),
            Command(Shortcut::Char('l'), Action::Fold(FoldAction::Expand)),
            Command(
                Shortcut::Code(KeyCode::Left),
                Action::Fold(FoldAction::Collapse),
            ),
            Command(
                Shortcut::Code(KeyCode::Right),
                Action::Fold(FoldAction::Expand),
            ),
        ],
    ),
    Binding::new(
        "Enter",
        "toggle the selected subtree's fold, including auto-folds",
        &[Command(
            Shortcut::Code(KeyCode::Enter),
            Action::Fold(FoldAction::Toggle),
        )],
    ),
    Binding::new(
        "za/zo/zc",
        "toggle, expand, or collapse the selected fold",
        &[
            Command(Shortcut::Seq('z', 'a'), Action::Fold(FoldAction::Toggle)),
            Command(Shortcut::Seq('z', 'o'), Action::Fold(FoldAction::Expand)),
            Command(Shortcut::Seq('z', 'c'), Action::Fold(FoldAction::Collapse)),
        ],
    )
    .hint("z…", "fold"),
    Binding::new(
        "zA/zO/zC",
        "toggle, expand, or collapse the selected fold recursively",
        &[
            Command(
                Shortcut::Seq('z', 'A'),
                Action::Fold(FoldAction::ToggleRecursive),
            ),
            Command(
                Shortcut::Seq('z', 'O'),
                Action::Fold(FoldAction::ExpandRecursive),
            ),
            Command(
                Shortcut::Seq('z', 'C'),
                Action::Fold(FoldAction::CollapseRecursive),
            ),
        ],
    ),
    Binding::new(
        "zR/zM",
        "expand or collapse every fold",
        &[
            Command(Shortcut::Seq('z', 'R'), Action::Fold(FoldAction::ExpandAll)),
            Command(
                Shortcut::Seq('z', 'M'),
                Action::Fold(FoldAction::CollapseAll),
            ),
        ],
    ),
    Binding::new(
        "zr/zm",
        "reveal or hide one tree level",
        &[
            Command(
                Shortcut::Seq('z', 'r'),
                Action::Fold(FoldAction::RevealLevel),
            ),
            Command(Shortcut::Seq('z', 'm'), Action::Fold(FoldAction::HideLevel)),
        ],
    ),
];

const PROCESS_ACTIONS: &[Binding<Action>] = &[
    Binding::new(
        "s",
        "cycle sort metric",
        &[Command(Shortcut::Char('s'), Action::CycleMetric)],
    )
    .hint("s", "sort"),
    Binding::new(
        "m",
        "toggle self vs self+children accounting",
        &[Command(Shortcut::Char('m'), Action::CycleScope)],
    )
    .hint("m", "mode"),
    Binding::new(
        "r",
        "force an immediate process memory rescan",
        &[Command(Shortcut::Char('r'), Action::Refresh)],
    )
    .hint("r", "rescan"),
    Binding::new(
        "K",
        "confirm SIGTERM for selected process",
        &[Command(Shortcut::Char('K'), Action::Kill)],
    )
    .hint("K", "SIGTERM"),
];

const TMPFS_ACTIONS: &[Binding<Action>] = &[
    Binding::new(
        "m",
        "toggle self vs self+children search context",
        &[Command(Shortcut::Char('m'), Action::CycleScope)],
    )
    .hint("m", "mode"),
    Binding::new(
        "r",
        "replace the complete tmpfs capture generation",
        &[Command(Shortcut::Char('r'), Action::Refresh)],
    )
    .hint("r", "refresh"),
];

const SHARED_ACTIONS: &[Binding<Action>] = &[
    Binding::new(
        "s",
        "cycle memory lens",
        &[Command(Shortcut::Char('s'), Action::CycleMetric)],
    )
    .hint("s", "sort"),
    Binding::new(
        "r",
        "force an immediate shared-object rescan",
        &[Command(Shortcut::Char('r'), Action::Refresh)],
    )
    .hint("r", "rescan"),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEvent, KeyModifiers};
    use tui_eternal::{Keymap, Resolution};

    fn plain(character: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE)
    }

    #[test]
    fn pane_catalog_is_the_dispatch_boundary() {
        let resolve =
            |tab: Tab, character| Keymap::default().resolve(&tab.sections(), plain(character));
        assert_eq!(
            resolve(Tab::Processes, 'm'),
            Resolution::Command(Action::CycleScope)
        );
        assert_eq!(resolve(Tab::Shared, 'm'), Resolution::Ignored);
        assert_eq!(
            resolve(Tab::Shared, 's'),
            Resolution::Command(Action::CycleMetric)
        );
        assert_eq!(resolve(Tab::Tmpfs, 's'), Resolution::Ignored);
    }

    #[test]
    fn keymap_resolves_declared_sequences_only_in_their_panes() {
        let mut keymap = Keymap::default();
        let mut resolve = |tab: Tab, character| keymap.resolve(&tab.sections(), plain(character));

        assert_eq!(resolve(Tab::Processes, 'z'), Resolution::Pending);
        assert_eq!(
            resolve(Tab::Processes, 'O'),
            Resolution::Command(Action::Fold(FoldAction::ExpandRecursive))
        );
        assert_eq!(resolve(Tab::Shared, 'g'), Resolution::Pending);
        assert_eq!(
            resolve(Tab::Shared, 'g'),
            Resolution::Command(Action::FirstRow)
        );
        assert_eq!(resolve(Tab::Shared, 'z'), Resolution::Ignored);
    }
}
