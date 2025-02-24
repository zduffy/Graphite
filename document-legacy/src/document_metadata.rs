use graph_craft::document::{DocumentNode, NodeId, NodeNetwork};
use graphene_core::renderer::ClickTarget;
use graphene_core::renderer::Quad;
use graphene_core::transform::Footprint;
use graphene_core::uuid::ManipulatorGroupId;

use glam::{DAffine2, DVec2};
use std::collections::{HashMap, HashSet};
use std::num::NonZeroU64;

#[derive(Debug, Clone)]
pub struct DocumentMetadata {
	upstream_transforms: HashMap<NodeId, (Footprint, DAffine2)>,
	structure: HashMap<LayerNodeIdentifier, NodeRelations>,
	artboards: HashSet<LayerNodeIdentifier>,
	folders: HashSet<LayerNodeIdentifier>,
	click_targets: HashMap<LayerNodeIdentifier, Vec<ClickTarget>>,
	selected_nodes: Vec<NodeId>,
	/// Transform from document space to viewport space.
	pub document_to_viewport: DAffine2,
}

impl Default for DocumentMetadata {
	fn default() -> Self {
		Self {
			upstream_transforms: HashMap::new(),
			click_targets: HashMap::new(),
			structure: HashMap::from_iter([(LayerNodeIdentifier::ROOT, NodeRelations::default())]),
			artboards: HashSet::new(),
			folders: HashSet::new(),
			selected_nodes: Vec::new(),
			document_to_viewport: DAffine2::IDENTITY,
		}
	}
}
pub struct SelectionChanged;

// layer iters
impl DocumentMetadata {
	/// Get the root layer from the document
	pub const fn root(&self) -> LayerNodeIdentifier {
		LayerNodeIdentifier::ROOT
	}

	pub fn all_layers(&self) -> DecendantsIter<'_> {
		self.root().decendants(self)
	}

	pub fn all_layers_except_artboards(&self) -> impl Iterator<Item = LayerNodeIdentifier> + '_ {
		self.all_layers().filter(move |layer| !self.artboards.contains(layer))
	}

	pub fn selected_layers(&self) -> impl Iterator<Item = LayerNodeIdentifier> + '_ {
		self.all_layers().filter(|layer| self.selected_nodes.contains(&layer.to_node()))
	}

	pub fn selected_layers_except_artboards(&self) -> impl Iterator<Item = LayerNodeIdentifier> + '_ {
		self.selected_layers().filter(move |layer| !self.artboards.contains(layer))
	}

	pub fn selected_layers_contains(&self, layer: LayerNodeIdentifier) -> bool {
		self.selected_layers().any(|selected| selected == layer)
	}

	pub fn selected_nodes(&self) -> core::slice::Iter<'_, NodeId> {
		self.selected_nodes.iter()
	}

	pub fn selected_nodes_ref(&self) -> &Vec<NodeId> {
		&self.selected_nodes
	}

	pub fn has_selected_nodes(&self) -> bool {
		!self.selected_nodes.is_empty()
	}

	pub fn layer_exists(&self, layer: LayerNodeIdentifier) -> bool {
		self.structure.contains_key(&layer)
	}

	pub fn click_target(&self, layer: LayerNodeIdentifier) -> Option<&Vec<ClickTarget>> {
		self.click_targets.get(&layer)
	}

	/// Access the [`NodeRelations`] of a layer.
	fn get_relations(&self, node_identifier: LayerNodeIdentifier) -> Option<&NodeRelations> {
		self.structure.get(&node_identifier)
	}

	/// Mutably access the [`NodeRelations`] of a layer.
	fn get_structure_mut(&mut self, node_identifier: LayerNodeIdentifier) -> &mut NodeRelations {
		self.structure.entry(node_identifier).or_default()
	}

	/// Layers excluding ones that are children of other layers in the list.
	pub fn shallowest_unique_layers(&self, layers: impl Iterator<Item = LayerNodeIdentifier>) -> Vec<Vec<LayerNodeIdentifier>> {
		let mut sorted_layers = layers
			.map(|layer| {
				let mut layer_path = layer.ancestors(self).collect::<Vec<_>>();
				layer_path.reverse();
				layer_path
			})
			.collect::<Vec<_>>();
		sorted_layers.sort();
		// Sorting here creates groups of similar UUID paths
		sorted_layers.dedup_by(|a, b| a.starts_with(b));
		sorted_layers
	}

	/// Ancestor that is shared by all layers and that is deepest (more nested). Default may be the root.
	pub fn deepest_common_ancestor(&self, layers: impl Iterator<Item = LayerNodeIdentifier>, include_self: bool) -> Option<LayerNodeIdentifier> {
		layers
			.map(|layer| {
				let mut layer_path = layer.ancestors(self).collect::<Vec<_>>();
				layer_path.reverse();

				if include_self || !self.folders.contains(&layer) {
					layer_path.pop();
				}

				layer_path
			})
			.reduce(|mut a, b| {
				a.truncate(a.iter().zip(b.iter()).position(|(&a, &b)| a != b).unwrap_or_else(|| a.len().min(b.len())));
				a
			})
			.and_then(|layer| layer.last().copied())
	}

	pub fn active_artboard(&self) -> LayerNodeIdentifier {
		self.artboards.iter().next().copied().unwrap_or(LayerNodeIdentifier::ROOT)
	}

	pub fn is_folder(&self, layer: LayerNodeIdentifier) -> bool {
		self.folders.contains(&layer)
	}

	pub fn is_artboard(&self, layer: LayerNodeIdentifier) -> bool {
		self.artboards.contains(&layer)
	}

	/// Filter out non folder layers
	pub fn folders<'a>(&'a self, layers: impl Iterator<Item = LayerNodeIdentifier> + 'a) -> impl Iterator<Item = LayerNodeIdentifier> + 'a {
		layers.filter(|layer| self.folders.contains(layer))
	}

	/// Folders sorted from most nested to least nested
	pub fn folders_sorted_by_most_nested(&self, layers: impl Iterator<Item = LayerNodeIdentifier>) -> Vec<LayerNodeIdentifier> {
		let mut folders: Vec<_> = self.folders(layers).collect();
		folders.sort_by_cached_key(|a| std::cmp::Reverse(a.ancestors(self).count()));
		folders
	}
}

// selected layer modifications
impl DocumentMetadata {
	#[must_use]
	pub fn retain_selected_nodes(&mut self, f: impl FnMut(&NodeId) -> bool) -> SelectionChanged {
		self.selected_nodes.retain(f);
		SelectionChanged
	}
	#[must_use]
	pub fn set_selected_nodes(&mut self, new: Vec<NodeId>) -> SelectionChanged {
		self.selected_nodes = new;
		SelectionChanged
	}
	#[must_use]
	pub fn add_selected_nodes(&mut self, iter: impl IntoIterator<Item = NodeId>) -> SelectionChanged {
		self.selected_nodes.extend(iter);
		SelectionChanged
	}
	#[must_use]
	pub fn clear_selected_nodes(&mut self) -> SelectionChanged {
		self.set_selected_nodes(Vec::new())
	}

	/// Loads the structure of layer nodes from a node graph.
	pub fn load_structure(&mut self, graph: &NodeNetwork) {
		self.structure = HashMap::from_iter([(LayerNodeIdentifier::ROOT, NodeRelations::default())]);
		self.folders = HashSet::new();
		self.artboards = HashSet::new();

		let id = graph.outputs[0].node_id;
		let Some(output_node) = graph.nodes.get(&id) else {
			return;
		};
		let Some((layer_node, node_id)) = first_child_layer(graph, output_node) else {
			return;
		};
		let parent = LayerNodeIdentifier::ROOT;
		let mut stack = vec![(layer_node, node_id, parent)];
		while let Some((node, id, parent)) = stack.pop() {
			let mut current = Some((node, id));
			while let Some(&(current_node, current_id)) = current.as_ref() {
				let current_identifier = LayerNodeIdentifier::new_unchecked(current_id);
				if !self.structure.contains_key(&current_identifier) {
					parent.push_child(self, current_identifier);

					if let Some((child_node, child_id)) = first_child_layer(graph, current_node) {
						stack.push((child_node, child_id, current_identifier));
					}

					if is_artboard(current_identifier, graph) {
						self.artboards.insert(current_identifier);
					}
					if is_folder(current_identifier, graph) {
						self.folders.insert(current_identifier);
					}
				}

				current = sibling_below(graph, current_node);
			}
		}

		self.selected_nodes.retain(|node| graph.nodes.contains_key(node));
		self.upstream_transforms.retain(|node, _| graph.nodes.contains_key(node));
		self.click_targets.retain(|layer, _| self.structure.contains_key(layer));
	}
}

fn first_child_layer<'a>(graph: &'a NodeNetwork, node: &DocumentNode) -> Option<(&'a DocumentNode, NodeId)> {
	graph.upstream_flow_back_from_nodes(vec![node.inputs[0].as_node()?], true).find(|(node, _)| node.is_layer())
}

fn sibling_below<'a>(graph: &'a NodeNetwork, node: &DocumentNode) -> Option<(&'a DocumentNode, NodeId)> {
	let construct_layer_node = &node.inputs[1];
	construct_layer_node.as_node().and_then(|id| graph.nodes.get(&id).filter(|node| node.is_layer()).map(|node| (node, id)))
}

// transforms
impl DocumentMetadata {
	/// Update the cached transforms of the layers
	pub fn update_transforms(&mut self, new_upstream_transforms: HashMap<NodeId, (Footprint, DAffine2)>) {
		self.upstream_transforms = new_upstream_transforms;
	}

	/// Access the cached transformation to document space from layer space
	pub fn transform_to_document(&self, layer: LayerNodeIdentifier) -> DAffine2 {
		self.document_to_viewport.inverse() * self.transform_to_viewport(layer)
	}

	pub fn transform_to_viewport(&self, layer: LayerNodeIdentifier) -> DAffine2 {
		layer
			.ancestors(self)
			.filter_map(|layer| self.upstream_transforms.get(&layer.to_node()))
			.copied()
			.map(|(footprint, transform)| footprint.transform * transform)
			.next()
			.unwrap_or(self.document_to_viewport)
	}

	pub fn upstream_transform(&self, node_id: NodeId) -> DAffine2 {
		self.upstream_transforms.get(&node_id).copied().map(|(_, transform)| transform).unwrap_or(DAffine2::IDENTITY)
	}

	pub fn downstream_transform_to_viewport(&self, layer: LayerNodeIdentifier) -> DAffine2 {
		self.upstream_transforms
			.get(&layer.to_node())
			.copied()
			.map(|(footprint, _)| footprint.transform)
			.unwrap_or_else(|| self.transform_to_viewport(layer))
	}
}

pub fn is_artboard(layer: LayerNodeIdentifier, network: &NodeNetwork) -> bool {
	network.upstream_flow_back_from_nodes(vec![layer.to_node()], true).any(|(node, _)| node.is_artboard())
}

pub fn is_folder(layer: LayerNodeIdentifier, network: &NodeNetwork) -> bool {
	network.nodes.get(&layer.to_node()).and_then(|node| node.inputs.first()).is_some_and(|input| input.as_node().is_none())
		|| network
			.upstream_flow_back_from_nodes(vec![layer.to_node()], true)
			.skip(1)
			.any(|(node, _)| node.is_artboard() || node.is_layer())
}

// click targets
impl DocumentMetadata {
	/// Update the cached click targets of the layers
	pub fn update_click_targets(&mut self, new_click_targets: HashMap<LayerNodeIdentifier, Vec<ClickTarget>>) {
		self.click_targets = new_click_targets;
	}

	/// Get the bounding box of the click target of the specified layer in the specified transform space
	pub fn bounding_box_with_transform(&self, layer: LayerNodeIdentifier, transform: DAffine2) -> Option<[DVec2; 2]> {
		self.click_targets
			.get(&layer)?
			.iter()
			.filter_map(|click_target| click_target.subpath.bounding_box_with_transform(transform))
			.reduce(Quad::combine_bounds)
	}

	/// Calculate the corners of the bounding box but with a nonzero size.
	///
	/// If the layer bounds are `0` in either axis then they are changed to be `1`.
	pub fn nonzero_bounding_box(&self, layer: LayerNodeIdentifier) -> [DVec2; 2] {
		let [bounds_min, mut bounds_max] = self.bounding_box_with_transform(layer, DAffine2::IDENTITY).unwrap_or_default();

		let bounds_size = bounds_max - bounds_min;
		if bounds_size.x < 1e-10 {
			bounds_max.x = bounds_min.x + 1.;
		}
		if bounds_size.y < 1e-10 {
			bounds_max.y = bounds_min.y + 1.;
		}

		[bounds_min, bounds_max]
	}

	/// Get the bounding box of the click target of the specified layer in document space
	pub fn bounding_box_document(&self, layer: LayerNodeIdentifier) -> Option<[DVec2; 2]> {
		self.bounding_box_with_transform(layer, self.transform_to_document(layer))
	}

	/// Get the bounding box of the click target of the specified layer in viewport space
	pub fn bounding_box_viewport(&self, layer: LayerNodeIdentifier) -> Option<[DVec2; 2]> {
		self.bounding_box_with_transform(layer, self.transform_to_viewport(layer))
	}

	/// Calculates the document bounds in viewport space
	pub fn document_bounds_viewport_space(&self) -> Option<[DVec2; 2]> {
		self.all_layers().filter_map(|layer| self.bounding_box_viewport(layer)).reduce(Quad::combine_bounds)
	}

	/// Calculates the document bounds in document space
	pub fn document_bounds_document_space(&self, include_artboards: bool) -> Option<[DVec2; 2]> {
		self.all_layers()
			.filter(|&layer| include_artboards || !self.is_artboard(layer))
			.filter_map(|layer| self.bounding_box_document(layer))
			.reduce(Quad::combine_bounds)
	}

	/// Calculates the selected layer bounds in document space
	pub fn selected_bounds_document_space(&self, include_artboards: bool) -> Option<[DVec2; 2]> {
		self.selected_layers()
			.filter(|&layer| include_artboards || !self.is_artboard(layer))
			.filter_map(|layer| self.bounding_box_document(layer))
			.reduce(Quad::combine_bounds)
	}

	pub fn layer_outline<'a>(&'a self, layer: LayerNodeIdentifier) -> impl Iterator<Item = &'a bezier_rs::Subpath<ManipulatorGroupId>> {
		static EMPTY: Vec<ClickTarget> = Vec::new();
		let click_targets = self.click_targets.get(&layer).unwrap_or(&EMPTY);
		click_targets.iter().map(|click_target| &click_target.subpath)
	}
}

/// Id of a layer node
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct LayerNodeIdentifier(NonZeroU64);

impl Default for LayerNodeIdentifier {
	fn default() -> Self {
		Self::ROOT
	}
}

impl core::fmt::Debug for LayerNodeIdentifier {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_tuple("LayerNodeIdentifier").field(&self.to_node()).finish()
	}
}

impl core::fmt::Display for LayerNodeIdentifier {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_fmt(format_args!("Layer(node_id={})", self.to_node()))
	}
}

impl LayerNodeIdentifier {
	pub const ROOT: Self = LayerNodeIdentifier::new_unchecked(0);

	/// Construct a [`LayerNodeIdentifier`] without checking if it is a layer node
	pub const fn new_unchecked(node_id: NodeId) -> Self {
		// Safety: will always be >=1
		Self(unsafe { NonZeroU64::new_unchecked(node_id + 1) })
	}

	/// Construct a [`LayerNodeIdentifier`], debug asserting that it is a layer node
	#[track_caller]
	pub fn new(node_id: NodeId, network: &NodeNetwork) -> Self {
		debug_assert!(
			is_layer_node(node_id, network),
			"Layer identifier constructed from non layer node {node_id}: {:#?}",
			network.nodes.get(&node_id)
		);
		Self::new_unchecked(node_id)
	}

	pub fn from_path(path: &[u64], network: &NodeNetwork) -> Self {
		Self::new(*path.last().unwrap(), network)
	}

	/// Access the node id of this layer
	pub fn to_node(self) -> NodeId {
		u64::from(self.0) - 1
	}

	/// Convert layer to layer path
	pub fn to_path(self) -> Vec<NodeId> {
		vec![self.to_node()]
	}

	/// Access the parent layer if possible
	pub fn parent(self, document_metadata: &DocumentMetadata) -> Option<LayerNodeIdentifier> {
		document_metadata.get_relations(self).and_then(|relations| relations.parent)
	}

	/// Access the previous sibling of this layer (up the layer tree)
	pub fn previous_sibling(self, document_metadata: &DocumentMetadata) -> Option<LayerNodeIdentifier> {
		document_metadata.get_relations(self).and_then(|relations| relations.previous_sibling)
	}

	/// Access the next sibling of this layer (down the layer tree)
	pub fn next_sibling(self, document_metadata: &DocumentMetadata) -> Option<LayerNodeIdentifier> {
		document_metadata.get_relations(self).and_then(|relations| relations.next_sibling)
	}

	/// Access the first child of this layer (top most in layer tree)
	pub fn first_child(self, document_metadata: &DocumentMetadata) -> Option<LayerNodeIdentifier> {
		document_metadata.get_relations(self).and_then(|relations| relations.first_child)
	}

	/// Access the last child of this layer (bottom most in layer tree)
	pub fn last_child(self, document_metadata: &DocumentMetadata) -> Option<LayerNodeIdentifier> {
		document_metadata.get_relations(self).and_then(|relations| relations.last_child)
	}

	/// Does the layer have children?
	pub fn has_children(self, document_metadata: &DocumentMetadata) -> bool {
		self.first_child(document_metadata).is_some()
	}

	/// Iterator over all direct children (excluding self and recursive children)
	pub fn children(self, document_metadata: &DocumentMetadata) -> AxisIter {
		AxisIter {
			layer_node: self.first_child(document_metadata),
			next_node: Self::next_sibling,
			document_metadata,
		}
	}

	/// All ancestors of this layer, including self, going to the document root
	pub fn ancestors(self, document_metadata: &DocumentMetadata) -> AxisIter {
		AxisIter {
			layer_node: Some(self),
			next_node: Self::parent,
			document_metadata,
		}
	}

	/// Iterator through all the last children, starting from self
	pub fn last_children(self, document_metadata: &DocumentMetadata) -> AxisIter {
		AxisIter {
			layer_node: Some(self),
			next_node: Self::last_child,
			document_metadata,
		}
	}

	/// Iterator through all decendants, including recursive children (not including self)
	pub fn decendants(self, document_metadata: &DocumentMetadata) -> DecendantsIter {
		DecendantsIter {
			front: self.first_child(document_metadata),
			back: self.last_child(document_metadata).and_then(|child| child.last_children(document_metadata).last()),
			document_metadata,
		}
	}

	/// Add a child towards the top of the layer tree
	pub fn push_front_child(self, document_metadata: &mut DocumentMetadata, new: LayerNodeIdentifier) {
		assert!(!document_metadata.structure.contains_key(&new), "Cannot add already existing layer");
		let parent = document_metadata.get_structure_mut(self);
		let old_first_child = parent.first_child.replace(new);
		parent.last_child.get_or_insert(new);
		if let Some(old_first_child) = old_first_child {
			document_metadata.get_structure_mut(old_first_child).previous_sibling = Some(new);
		}
		document_metadata.get_structure_mut(new).next_sibling = old_first_child;
		document_metadata.get_structure_mut(new).parent = Some(self);
	}

	/// Add a child towards the bottom of the layer tree
	pub fn push_child(self, document_metadata: &mut DocumentMetadata, new: LayerNodeIdentifier) {
		assert!(!document_metadata.structure.contains_key(&new), "Cannot add already existing layer");
		let parent = document_metadata.get_structure_mut(self);
		let old_last_child = parent.last_child.replace(new);
		parent.first_child.get_or_insert(new);
		if let Some(old_last_child) = old_last_child {
			document_metadata.get_structure_mut(old_last_child).next_sibling = Some(new);
		}
		document_metadata.get_structure_mut(new).previous_sibling = old_last_child;
		document_metadata.get_structure_mut(new).parent = Some(self);
	}

	/// Add sibling above in the layer tree
	pub fn add_before(self, document_metadata: &mut DocumentMetadata, new: LayerNodeIdentifier) {
		assert!(!document_metadata.structure.contains_key(&new), "Cannot add already existing layer");
		document_metadata.get_structure_mut(new).next_sibling = Some(self);
		document_metadata.get_structure_mut(new).parent = self.parent(document_metadata);
		let old_previous_sibling = document_metadata.get_structure_mut(self).previous_sibling.replace(new);
		if let Some(old_previous_sibling) = old_previous_sibling {
			document_metadata.get_structure_mut(old_previous_sibling).next_sibling = Some(new);
			document_metadata.get_structure_mut(new).previous_sibling = Some(old_previous_sibling);
		} else if let Some(structure) = self
			.parent(document_metadata)
			.map(|parent| document_metadata.get_structure_mut(parent))
			.filter(|structure| structure.first_child == Some(self))
		{
			structure.first_child = Some(new);
		}
	}

	/// Add sibling below in the layer tree
	pub fn add_after(self, document_metadata: &mut DocumentMetadata, new: LayerNodeIdentifier) {
		assert!(!document_metadata.structure.contains_key(&new), "Cannot add already existing layer");
		document_metadata.get_structure_mut(new).previous_sibling = Some(self);
		document_metadata.get_structure_mut(new).parent = self.parent(document_metadata);
		let old_next_sibling = document_metadata.get_structure_mut(self).next_sibling.replace(new);
		if let Some(old_next_sibling) = old_next_sibling {
			document_metadata.get_structure_mut(old_next_sibling).previous_sibling = Some(new);
			document_metadata.get_structure_mut(new).next_sibling = Some(old_next_sibling);
		} else if let Some(structure) = self
			.parent(document_metadata)
			.map(|parent| document_metadata.get_structure_mut(parent))
			.filter(|structure| structure.last_child == Some(self))
		{
			structure.last_child = Some(new);
		}
	}

	/// Delete layer and all children
	pub fn delete(self, document_metadata: &mut DocumentMetadata) {
		let previous_sibling = self.previous_sibling(document_metadata);
		let next_sibling = self.next_sibling(document_metadata);

		if let Some(previous_sibling) = previous_sibling.map(|node| document_metadata.get_structure_mut(node)) {
			previous_sibling.next_sibling = next_sibling;
		}

		if let Some(next_sibling) = next_sibling.map(|node| document_metadata.get_structure_mut(node)) {
			next_sibling.previous_sibling = previous_sibling;
		}
		let mut parent = self.parent(document_metadata).map(|parent| document_metadata.get_structure_mut(parent));
		if let Some(structure) = parent.as_mut().filter(|structure| structure.first_child == Some(self)) {
			structure.first_child = next_sibling;
		}
		if let Some(structure) = parent.as_mut().filter(|structure| structure.last_child == Some(self)) {
			structure.last_child = previous_sibling;
		}

		let mut delete = vec![self];
		delete.extend(self.decendants(document_metadata));
		for node in delete {
			document_metadata.structure.remove(&node);
		}
	}

	pub fn exists(&self, document_metadata: &DocumentMetadata) -> bool {
		document_metadata.get_relations(*self).is_some()
	}

	pub fn starts_with(&self, other: Self, document_metadata: &DocumentMetadata) -> bool {
		self.ancestors(document_metadata).any(|parent| parent == other)
	}

	pub fn child_of_root(&self, document_metadata: &DocumentMetadata) -> Self {
		self.ancestors(document_metadata)
			.filter(|&layer| layer != LayerNodeIdentifier::ROOT)
			.last()
			.expect("There should be a layer before the root")
	}
}

impl From<NodeId> for LayerNodeIdentifier {
	fn from(node_id: NodeId) -> Self {
		Self::new_unchecked(node_id)
	}
}

impl From<LayerNodeIdentifier> for NodeId {
	fn from(identifier: LayerNodeIdentifier) -> Self {
		identifier.to_node()
	}
}

/// Iterator over specified axis.
#[derive(Clone)]
pub struct AxisIter<'a> {
	layer_node: Option<LayerNodeIdentifier>,
	next_node: fn(LayerNodeIdentifier, &DocumentMetadata) -> Option<LayerNodeIdentifier>,
	document_metadata: &'a DocumentMetadata,
}

impl<'a> Iterator for AxisIter<'a> {
	type Item = LayerNodeIdentifier;

	fn next(&mut self) -> Option<Self::Item> {
		let layer_node = self.layer_node.take();
		self.layer_node = layer_node.and_then(|node| (self.next_node)(node, self.document_metadata));
		layer_node
	}
}

#[derive(Clone)]
pub struct DecendantsIter<'a> {
	front: Option<LayerNodeIdentifier>,
	back: Option<LayerNodeIdentifier>,
	document_metadata: &'a DocumentMetadata,
}

impl<'a> Iterator for DecendantsIter<'a> {
	type Item = LayerNodeIdentifier;

	fn next(&mut self) -> Option<Self::Item> {
		if self.front == self.back {
			self.back = None;
			self.front.take()
		} else {
			let layer_node = self.front.take();
			if let Some(layer_node) = layer_node {
				self.front = layer_node
					.first_child(self.document_metadata)
					.or_else(|| layer_node.ancestors(self.document_metadata).find_map(|ancestor| ancestor.next_sibling(self.document_metadata)));
			}
			layer_node
		}
	}
}
impl<'a> DoubleEndedIterator for DecendantsIter<'a> {
	fn next_back(&mut self) -> Option<Self::Item> {
		if self.front == self.back {
			self.front = None;
			self.back.take()
		} else {
			let layer_node = self.back.take();
			if let Some(layer_node) = layer_node {
				self.back = layer_node
					.previous_sibling(self.document_metadata)
					.and_then(|sibling| sibling.last_children(self.document_metadata).last())
					.or_else(|| layer_node.parent(self.document_metadata));
			}

			layer_node
		}
	}
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NodeRelations {
	parent: Option<LayerNodeIdentifier>,
	previous_sibling: Option<LayerNodeIdentifier>,
	next_sibling: Option<LayerNodeIdentifier>,
	first_child: Option<LayerNodeIdentifier>,
	last_child: Option<LayerNodeIdentifier>,
}

fn is_layer_node(node: NodeId, network: &NodeNetwork) -> bool {
	node == LayerNodeIdentifier::ROOT.to_node() || network.nodes.get(&node).is_some_and(|node| node.is_layer())
}

#[test]
fn test_tree() {
	let mut document_metadata = DocumentMetadata::default();
	let root = document_metadata.root();
	let document_metadata = &mut document_metadata;
	root.push_child(document_metadata, LayerNodeIdentifier::new_unchecked(3));
	assert_eq!(root.children(document_metadata).collect::<Vec<_>>(), vec![LayerNodeIdentifier::new_unchecked(3)]);
	root.push_child(document_metadata, LayerNodeIdentifier::new_unchecked(6));
	assert_eq!(root.children(document_metadata).map(LayerNodeIdentifier::to_node).collect::<Vec<_>>(), vec![3, 6]);
	assert_eq!(root.decendants(document_metadata).map(LayerNodeIdentifier::to_node).collect::<Vec<_>>(), vec![3, 6]);
	LayerNodeIdentifier::new_unchecked(3).add_after(document_metadata, LayerNodeIdentifier::new_unchecked(4));
	LayerNodeIdentifier::new_unchecked(3).add_before(document_metadata, LayerNodeIdentifier::new_unchecked(2));
	LayerNodeIdentifier::new_unchecked(6).add_before(document_metadata, LayerNodeIdentifier::new_unchecked(5));
	LayerNodeIdentifier::new_unchecked(6).add_after(document_metadata, LayerNodeIdentifier::new_unchecked(9));
	LayerNodeIdentifier::new_unchecked(6).push_child(document_metadata, LayerNodeIdentifier::new_unchecked(8));
	LayerNodeIdentifier::new_unchecked(6).push_front_child(document_metadata, LayerNodeIdentifier::new_unchecked(7));
	root.push_front_child(document_metadata, LayerNodeIdentifier::new_unchecked(1));
	assert_eq!(root.children(document_metadata).map(LayerNodeIdentifier::to_node).collect::<Vec<_>>(), vec![1, 2, 3, 4, 5, 6, 9]);
	assert_eq!(
		root.decendants(document_metadata).map(LayerNodeIdentifier::to_node).collect::<Vec<_>>(),
		vec![1, 2, 3, 4, 5, 6, 7, 8, 9]
	);
	assert_eq!(
		root.decendants(document_metadata).map(LayerNodeIdentifier::to_node).rev().collect::<Vec<_>>(),
		vec![9, 8, 7, 6, 5, 4, 3, 2, 1]
	);
	assert!(root.children(document_metadata).all(|child| child.parent(document_metadata) == Some(root)));
	LayerNodeIdentifier::new_unchecked(6).delete(document_metadata);
	LayerNodeIdentifier::new_unchecked(1).delete(document_metadata);
	LayerNodeIdentifier::new_unchecked(9).push_child(document_metadata, LayerNodeIdentifier::new_unchecked(10));
	assert_eq!(root.children(document_metadata).map(LayerNodeIdentifier::to_node).collect::<Vec<_>>(), vec![2, 3, 4, 5, 9]);
	assert_eq!(root.decendants(document_metadata).map(LayerNodeIdentifier::to_node).collect::<Vec<_>>(), vec![2, 3, 4, 5, 9, 10]);
	assert_eq!(root.decendants(document_metadata).map(LayerNodeIdentifier::to_node).rev().collect::<Vec<_>>(), vec![10, 9, 5, 4, 3, 2]);
}
