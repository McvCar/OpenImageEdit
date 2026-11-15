use crate::sections::image_resources_section::DescriptorStructure;

/// An image resource from the image resources section
#[derive(Debug)]
#[allow(missing_docs)]
pub enum ImageResource {
    Slices(SlicesImageResource),
    /// Unparsed payload of a resource the caller wants to interpret itself.
    ///
    /// Patterns (`2491`..`2493`) live here: their channel data needs the same
    /// PackBits/RLE machinery as layer channels, which belongs to the decoder.
    Raw(RawImageResource),
}

/// A resource block kept as raw bytes.
#[derive(Debug)]
pub struct RawImageResource {
    pub(crate) resource_id: i16,
    pub(crate) name: String,
    pub(crate) data: Vec<u8>,
}

impl RawImageResource {
    /// The resource's unique identifier (e.g. `2491` for patterns).
    pub fn resource_id(&self) -> i16 {
        self.resource_id
    }

    /// The resource's Pascal name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The raw payload, without the block header or padding.
    pub fn data(&self) -> &[u8] {
        &self.data
    }
}

/// Comes from a slices resource block
#[derive(Debug)]
pub struct SlicesImageResource {
    pub(crate) name: String,
    pub(crate) descriptors: Vec<DescriptorStructure>,
}

#[allow(missing_docs)]
impl SlicesImageResource {
    pub fn name(&self) -> &String {
        &self.name
    }

    pub fn descriptors(&self) -> &Vec<DescriptorStructure> {
        &self.descriptors
    }
}
