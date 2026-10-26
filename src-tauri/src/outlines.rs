use std::collections::{HashMap, HashSet};

use lopdf::{Dictionary, Document, Object, ObjectId, dictionary, text_string};
use serde::Serialize;

use crate::{
    document::{Result, invalid, page_id},
    pdf_objects::{key, parse_key, resolve, text, validate_text},
};

const MAX_BOOKMARKS: usize = 2000;
const MAX_DEPTH: usize = 32;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BookmarkInfo {
    pub id: String,
    pub title: String,
    pub parent: Option<String>,
    pub depth: usize,
    pub position: usize,
    pub child_count: usize,
    pub page: Option<usize>,
    pub named_destination: Option<String>,
    pub url: Option<String>,
    pub target_description: String,
}

struct Node {
    dictionary: Dictionary,
    parent: Option<ObjectId>,
    children: Vec<ObjectId>,
}

struct Tree {
    root_id: Option<ObjectId>,
    root: Dictionary,
    children: Vec<ObjectId>,
    nodes: HashMap<ObjectId, Node>,
}

fn reference(dictionary: &Dictionary, name: &[u8]) -> Result<Option<ObjectId>> {
    dictionary
        .get(name)
        .ok()
        .map(|value| Ok(value.as_reference()?))
        .transpose()
}

impl Tree {
    fn read(document: &Document) -> Result<Self> {
        let Some(value) = document.catalog()?.get(b"Outlines").ok() else {
            return Ok(Self {
                root_id: None,
                root: dictionary! { "Type" => "Outlines" },
                children: Vec::new(),
                nodes: HashMap::new(),
            });
        };
        let root_id = value.as_reference().ok();
        let root = resolve(document, value)?.as_dict()?.clone();
        let mut nodes = HashMap::new();
        let mut visited = HashSet::new();
        if let Some(id) = root_id {
            visited.insert(id);
        }
        let children = read_siblings(document, &root, root_id, None, 0, &mut nodes, &mut visited)?;
        Ok(Self {
            root_id,
            root,
            children,
            nodes,
        })
    }

    fn children_mut(&mut self, parent: Option<ObjectId>) -> Result<&mut Vec<ObjectId>> {
        match parent {
            None => Ok(&mut self.children),
            Some(id) => self
                .nodes
                .get_mut(&id)
                .map(|node| &mut node.children)
                .ok_or_else(|| invalid("That parent bookmark no longer exists.")),
        }
    }

    fn write(mut self, document: &mut Document) -> Result<()> {
        let root_id = self
            .root_id
            .unwrap_or_else(|| document.add_object(self.root.clone()));
        let count = write_siblings(document, &self.nodes, &self.children, root_id, 0)?;
        set_children(&mut self.root, &self.children);
        self.root.set("Count", count as i64);
        document
            .objects
            .insert(root_id, Object::Dictionary(self.root));
        document.catalog_mut()?.set("Outlines", root_id);
        Ok(())
    }
}

fn read_siblings(
    document: &Document,
    container: &Dictionary,
    container_id: Option<ObjectId>,
    parent: Option<ObjectId>,
    depth: usize,
    nodes: &mut HashMap<ObjectId, Node>,
    visited: &mut HashSet<ObjectId>,
) -> Result<Vec<ObjectId>> {
    if depth > MAX_DEPTH {
        return Err(invalid("The bookmark tree exceeds 32 nested levels."));
    }
    let mut current = reference(container, b"First")?;
    let mut siblings = Vec::new();
    while let Some(id) = current {
        if !visited.insert(id) || nodes.len() + visited.len() > MAX_BOOKMARKS * 2 + 1 {
            return Err(invalid(
                "The bookmark tree is cyclic, shared, or exceeds 2,000 entries.",
            ));
        }
        let dictionary = document.get_dictionary(id)?.clone();
        if let (Some(actual), Some(expected)) = (reference(&dictionary, b"Parent")?, container_id)
            && actual != expected
        {
            return Err(invalid(
                "A bookmark has an inconsistent parent. Repair its outline in another reader first.",
            ));
        }
        if let Some(previous) = reference(&dictionary, b"Prev")?
            && siblings.last().copied() != Some(previous)
        {
            return Err(invalid("The bookmark sibling links are inconsistent."));
        }
        let children = read_siblings(
            document,
            &dictionary,
            Some(id),
            Some(id),
            depth + 1,
            nodes,
            visited,
        )?;
        current = reference(&dictionary, b"Next")?;
        nodes.insert(
            id,
            Node {
                dictionary,
                parent,
                children,
            },
        );
        siblings.push(id);
        if nodes.len() > MAX_BOOKMARKS {
            return Err(invalid("This PDF exceeds 2,000 bookmarks."));
        }
    }
    if reference(container, b"Last")? != siblings.last().copied() {
        return Err(invalid(
            "The bookmark tree has an inconsistent last-child link.",
        ));
    }
    Ok(siblings)
}

fn set_children(dictionary: &mut Dictionary, children: &[ObjectId]) {
    for key in [b"First".as_slice(), b"Last"] {
        dictionary.remove(key);
    }
    if let (Some(first), Some(last)) = (children.first(), children.last()) {
        dictionary.set("First", *first);
        dictionary.set("Last", *last);
    }
}

fn write_siblings(
    document: &mut Document,
    nodes: &HashMap<ObjectId, Node>,
    siblings: &[ObjectId],
    parent: ObjectId,
    depth: usize,
) -> Result<usize> {
    if depth > MAX_DEPTH {
        return Err(invalid("Organize bookmarks within 32 nested levels."));
    }
    let mut visible = 0;
    for (index, id) in siblings.iter().enumerate() {
        let node = nodes
            .get(id)
            .ok_or_else(|| invalid("A bookmark no longer exists."))?;
        let child_count = write_siblings(document, nodes, &node.children, *id, depth + 1)?;
        let mut dictionary = node.dictionary.clone();
        let closed = dictionary
            .get(b"Count")
            .and_then(Object::as_i64)
            .is_ok_and(|value| value < 0);
        for key in [b"Prev".as_slice(), b"Next", b"Count"] {
            dictionary.remove(key);
        }
        dictionary.set("Parent", parent);
        if index > 0 {
            dictionary.set("Prev", siblings[index - 1]);
        }
        if let Some(next) = siblings.get(index + 1) {
            dictionary.set("Next", *next);
        }
        set_children(&mut dictionary, &node.children);
        if child_count > 0 {
            dictionary.set(
                "Count",
                if closed {
                    -(child_count as i64)
                } else {
                    child_count as i64
                },
            );
        }
        document.objects.insert(*id, Object::Dictionary(dictionary));
        visible += 1 + if closed { 0 } else { child_count };
    }
    Ok(visible)
}

fn target(
    document: &Document,
    dictionary: &Dictionary,
) -> Result<(Option<usize>, Option<String>, Option<String>)> {
    let destination = if let Ok(value) = dictionary.get(b"Dest") {
        Some(resolve(document, value)?)
    } else if let Ok(value) = dictionary.get(b"A") {
        let action = resolve(document, value)?.as_dict()?;
        match action.get(b"S").and_then(Object::as_name).ok() {
            Some(b"GoTo") => action
                .get(b"D")
                .ok()
                .map(|value| resolve(document, value))
                .transpose()?,
            Some(b"URI") => {
                return Ok((None, None, Some(text(document, action.get(b"URI")?, 8192)?)));
            }
            _ => None,
        }
    } else {
        None
    };
    match destination {
        Some(Object::Array(array)) if !array.is_empty() => {
            let page = match &array[0] {
                Object::Reference(id) => document.get_pages().values().position(|page| page == id),
                Object::Integer(index) if *index >= 0 => usize::try_from(*index)
                    .ok()
                    .filter(|index| *index < document.get_pages().len()),
                _ => None,
            };
            Ok((page, None, None))
        }
        Some(Object::String(..)) => Ok((
            None,
            Some(text(document, destination.unwrap(), 4096)?),
            None,
        )),
        Some(Object::Name(name)) => {
            let value = std::str::from_utf8(name)
                .map_err(|_| invalid("A named bookmark destination is not UTF-8."))?;
            if value.len() > 4096 {
                return Err(invalid("A named bookmark destination is too long."));
            }
            Ok((None, Some(value.to_owned()), None))
        }
        _ => Ok((None, None, None)),
    }
}

pub(crate) fn inspect(document: &Document) -> Result<Vec<BookmarkInfo>> {
    let tree = Tree::read(document)?;
    fn flatten(
        document: &Document,
        tree: &Tree,
        ids: &[ObjectId],
        depth: usize,
        output: &mut Vec<BookmarkInfo>,
    ) -> Result<()> {
        for (position, id) in ids.iter().enumerate() {
            let node = &tree.nodes[id];
            let title = text(document, node.dictionary.get(b"Title")?, 4096)?;
            let (page, named_destination, url) = target(document, &node.dictionary)?;
            let target_description = if let Some(page) = page {
                format!("Page {}", page + 1)
            } else if named_destination.is_some() {
                "Named PDF destination".into()
            } else if url.is_some() {
                "External link (confirmation required)".into()
            } else {
                "Other or unresolved PDF target (preserved)".into()
            };
            output.push(BookmarkInfo {
                id: key(*id),
                title,
                parent: node.parent.map(key),
                depth,
                position,
                child_count: node.children.len(),
                page,
                named_destination,
                url,
                target_description,
            });
            flatten(document, tree, &node.children, depth + 1, output)?;
        }
        Ok(())
    }
    let mut output = Vec::new();
    flatten(document, &tree, &tree.children, 0, &mut output)?;
    Ok(output)
}

pub(crate) fn create(
    document: &mut Document,
    parent: Option<String>,
    at: usize,
    title: &str,
    page: usize,
) -> Result<()> {
    validate_text(title, 4096, false)?;
    let page = page_id(document, page)?;
    let mut tree = Tree::read(document)?;
    if tree.nodes.len() >= MAX_BOOKMARKS {
        return Err(invalid("This PDF already has 2,000 bookmarks."));
    }
    let parent = parent.as_deref().map(parse_key).transpose()?;
    if at > tree.children_mut(parent)?.len() {
        return Err(invalid("Choose a valid bookmark position."));
    }
    let dictionary = dictionary! { "Title" => text_string(title), "Dest" => vec![page.into(), Object::Name(b"Fit".to_vec())] };
    let id = document.add_object(dictionary.clone());
    tree.nodes.insert(
        id,
        Node {
            dictionary,
            parent,
            children: Vec::new(),
        },
    );
    tree.children_mut(parent)?.insert(at, id);
    tree.write(document)
}

pub(crate) fn rename(document: &mut Document, id: &str, title: &str) -> Result<()> {
    validate_text(title, 4096, false)?;
    let mut tree = Tree::read(document)?;
    tree.nodes
        .get_mut(&parse_key(id)?)
        .ok_or_else(|| invalid("That bookmark no longer exists."))?
        .dictionary
        .set("Title", text_string(title));
    tree.write(document)
}

pub(crate) fn move_bookmark(
    document: &mut Document,
    id: &str,
    parent: Option<String>,
    at: usize,
) -> Result<()> {
    let mut tree = Tree::read(document)?;
    let id = parse_key(id)?;
    let parent = parent.as_deref().map(parse_key).transpose()?;
    let old_parent = tree
        .nodes
        .get(&id)
        .ok_or_else(|| invalid("That bookmark no longer exists."))?
        .parent;
    let mut ancestor = parent;
    while let Some(current) = ancestor {
        if current == id {
            return Err(invalid(
                "A bookmark cannot be moved inside itself or its descendants.",
            ));
        }
        ancestor = tree
            .nodes
            .get(&current)
            .ok_or_else(|| invalid("That parent bookmark no longer exists."))?
            .parent;
    }
    tree.children_mut(old_parent)?.retain(|child| *child != id);
    let children = tree.children_mut(parent)?;
    if at > children.len() {
        return Err(invalid(
            "Choose a valid bookmark position after removing the moved bookmark.",
        ));
    }
    children.insert(at, id);
    tree.nodes
        .get_mut(&id)
        .ok_or_else(|| invalid("That bookmark no longer exists."))?
        .parent = parent;
    tree.write(document)
}

pub(crate) fn remove(document: &mut Document, id: &str) -> Result<()> {
    let mut tree = Tree::read(document)?;
    let id = parse_key(id)?;
    let parent = tree
        .nodes
        .get(&id)
        .ok_or_else(|| invalid("That bookmark no longer exists."))?
        .parent;
    tree.children_mut(parent)?.retain(|child| *child != id);
    tree.write(document)
}
