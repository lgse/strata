//! The archive's member tree as data, and the folder shown as an [`EntrySource`].

use std::collections::HashMap;

use crate::contract::{Entry, EntryId, EntryKind, EntrySource};

/// One member: a folder, or a file with its size and icon.
#[derive(Clone, Debug, PartialEq)]
pub enum ArchiveItem {
    Dir(ArchiveDir),
    File {
        name: String,
        size: u64,
        /// The icon's catalog name, from the host's file-name mapping (empty: the Look's File
        /// icon).
        icon: String,
    },
}

/// A folder in the archive, its children in the order to show (Strata's lister puts folders
/// first, then files, each by name; the order is the host's).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ArchiveDir {
    /// Empty for the archive's root.
    pub name: String,
    pub children: Vec<ArchiveItem>,
}

#[derive(Clone, Debug)]
struct Node {
    id: EntryId,
    /// What the row shows: a folder's name ends in "/", since an archive may hold a file and
    /// a folder of the same name.
    shown: String,
    /// The name as the crumbs show it.
    name: String,
    kind: EntryKind,
    icon: String,
    size: u64,
    parent: usize,
    /// Its place among its parent's children.
    index: usize,
    children: Vec<usize>,
}

/// The member tree, flattened, with an id per member.
#[derive(Clone, Debug)]
pub struct ArchiveTree {
    /// Node 0 is the root.
    nodes: Vec<Node>,
    by_id: HashMap<EntryId, usize>,
}

impl ArchiveTree {
    /// The tree under `root`, with an id from `alloc` for every member.
    ///
    /// The ids follow W1's [`EntryId`] contract: `alloc` hands out ids unique across every
    /// source the process shows at once (the listing, every pane, column and window), each of
    /// which lives for the process and is never handed out again, even after this tree is
    /// dropped. Strata reserves them from its process-wide table; the gallery reserves a high
    /// range of its own.
    pub fn new(root: ArchiveDir, mut alloc: impl FnMut() -> EntryId) -> ArchiveTree {
        let mut tree = ArchiveTree {
            nodes: vec![Node {
                id: EntryId::new(u64::MAX),
                shown: String::new(),
                name: String::new(),
                kind: EntryKind::Folder,
                icon: String::new(),
                size: 0,
                parent: 0,
                index: 0,
                children: Vec::new(),
            }],
            by_id: HashMap::new(),
        };
        tree.add_children(0, root.children, &mut alloc);
        tree
    }

    fn add_children(
        &mut self,
        parent: usize,
        children: Vec<ArchiveItem>,
        alloc: &mut impl FnMut() -> EntryId,
    ) {
        for (index, item) in children.into_iter().enumerate() {
            let at = self.nodes.len();
            let id = alloc();
            let (node, grandchildren) = match item {
                ArchiveItem::Dir(dir) => (
                    Node {
                        id,
                        shown: format!("{}/", dir.name),
                        name: dir.name,
                        kind: EntryKind::Folder,
                        icon: String::new(),
                        size: 0,
                        parent,
                        index,
                        children: Vec::new(),
                    },
                    Some(dir.children),
                ),
                ArchiveItem::File { name, size, icon } => (
                    Node {
                        id,
                        shown: name.clone(),
                        name,
                        kind: EntryKind::File,
                        icon,
                        size,
                        parent,
                        index,
                        children: Vec::new(),
                    },
                    None,
                ),
            };
            self.nodes.push(node);
            self.by_id.insert(id, at);
            self.nodes[parent].children.push(at);
            if let Some(grandchildren) = grandchildren {
                self.add_children(at, grandchildren, alloc);
            }
        }
    }

    fn child(&self, dir: usize, index: usize) -> Option<usize> {
        self.nodes.get(dir)?.children.get(index).copied()
    }
}

/// The folder shown, as the collection reads it.
#[derive(Clone, Debug)]
pub struct ArchiveListing {
    tree: ArchiveTree,
    dir: usize,
}

impl ArchiveListing {
    pub(super) fn new(tree: ArchiveTree) -> ArchiveListing {
        ArchiveListing { tree, dir: 0 }
    }

    /// How deep the folder shown is: 0 at the root.
    pub fn depth(&self) -> usize {
        self.path().len()
    }

    /// The folders from the root down to the one shown, by node, the root left out.
    fn path(&self) -> Vec<usize> {
        let mut path = Vec::new();
        let mut at = self.dir;
        while at != 0 {
            path.push(at);
            at = self.tree.nodes[at].parent;
        }
        path.reverse();
        path
    }

    /// The crumbs' names below the root, down to the folder shown.
    pub fn crumbs(&self) -> Vec<&str> {
        self.path()
            .into_iter()
            .map(|n| self.tree.nodes[n].name.as_str())
            .collect()
    }

    /// Whether entry `index` of the folder shown is a folder.
    pub fn is_dir(&self, index: usize) -> bool {
        self.tree
            .child(self.dir, index)
            .is_some_and(|n| self.tree.nodes[n].kind.is_folder_like())
    }

    /// The size of the file at `index`.
    pub fn size(&self, index: usize) -> Option<u64> {
        let n = self.tree.child(self.dir, index)?;
        let node = &self.tree.nodes[n];
        (!node.kind.is_folder_like()).then_some(node.size)
    }

    /// Files and folders in the folder shown.
    pub fn counts(&self) -> (usize, usize) {
        let children = &self.tree.nodes[self.dir].children;
        let folders = children
            .iter()
            .filter(|&&n| self.tree.nodes[n].kind.is_folder_like())
            .count();
        (children.len() - folders, folders)
    }

    /// Shows the folder at `index`; false if it isn't one.
    pub(super) fn open(&mut self, index: usize) -> bool {
        match self.tree.child(self.dir, index) {
            Some(n) if self.tree.nodes[n].kind.is_folder_like() => {
                self.dir = n;
                true
            }
            _ => false,
        }
    }

    /// Shows the folder `depth` levels below the root, on the way to the one shown. Returns
    /// where the folder left sits in it, for the cursor.
    pub(super) fn go_to(&mut self, depth: usize) -> Option<usize> {
        let path = self.path();
        if depth >= path.len() {
            return None;
        }
        let from = path[depth];
        self.dir = self.tree.nodes[from].parent;
        Some(self.tree.nodes[from].index)
    }
}

impl EntrySource for ArchiveListing {
    fn len(&self) -> usize {
        self.tree.nodes[self.dir].children.len()
    }

    fn id_at(&self, index: usize) -> EntryId {
        self.tree.nodes[self.tree.nodes[self.dir].children[index]].id
    }

    fn index_of(&self, id: EntryId) -> Option<usize> {
        let &n = self.tree.by_id.get(&id)?;
        let node = &self.tree.nodes[n];
        (node.parent == self.dir).then_some(node.index)
    }

    fn entry(&self, index: usize) -> Entry<'_> {
        let node = &self.tree.nodes[self.tree.nodes[self.dir].children[index]];
        Entry::new(node.id, &node.shown, node.kind, &node.icon)
    }
}
