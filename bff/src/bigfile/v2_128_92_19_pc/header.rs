use std::io::{Write, Seek, SeekFrom};
use binrw::{BinRead, BinWrite, binread, Endian, BinResult};

use crate::bigfile::v1_06_63_02_pc::header::BigFileType;
use crate::bigfile::versions::VersionOneple;
use crate::helpers::DynArray;
use crate::names::Name;

#[derive(Debug, BinRead)]
pub struct DataDescription {
    pub resource_count: u32,
    pub padded_size: u64,
    pub size: u64,
    pub working_buffer_offset: u64,
}

impl DataDescription {
    const SIZE: u64 = 28;
}

#[derive(Debug, BinRead)]
pub struct Resource {
    pub name: Name,
    pub class_name: Name,
    pub offset: u32,
    pub size: u64,
    pub decompressed_size: u64,
}

#[binread]
#[derive(Debug)]
pub struct Resources {
    #[br(temp)]
    pub data_count: u32,
    pub data_offset: u32,
    pub working_buffer_offset: u32,
    pub unk1: u32,
    pub unk2: u64,
    pub padded_size: u64,
    pub padding_size: u64,
    #[br(count = data_count, pad_after = DataDescription::SIZE * 52 - DataDescription::SIZE * data_count as u64)]
    pub data_descriptions: Vec<DataDescription>,
    // Use a Vec here instead of DynArray because Resource doesn't impl BinWrite and binrw isn't smart with trait bounds
    #[br(temp)]
    pub resource_count: u32,
    #[br(count = resource_count)]
    pub resources: Vec<Resource>,
    #[br(align_after = 2048)]
    pub unk3: u64,
}

#[derive(Debug, BinRead, BinWrite)]
pub struct BlockDescription {
    pub unk1: u64,
    pub unk2: u64,
    pub unk3: u64,
    pub resources_map_offset: u32,
    pub data_resources_map_offset: u32,
}

#[derive(Debug, BinRead, BinWrite)]
pub struct Header {
    pub version_oneple: VersionOneple,
    pub bigfile_type: BigFileType,
    pub block_description_offset: u32,
    pub unk1: u32,
    pub pool_offset: u32,
    pub unk3: u32,
    pub unk4: u32,
    pub unk5: u32,
    pub total_padded_block_size: u64,
    pub block_sector_padding_size: u64,
    pub file_size: u64,
    pub total_decompressed_size: u64,
    pub zero: u64,
    #[brw(align_after = 4096)]
    pub total_resource_count: u32,
    // manually write, so bw(ignore)
    #[bw(ignore)]
    #[br(seek_before = SeekFrom::Start(block_description_offset as u64 * 2048))]
    pub block_descriptions: DynArray<BlockDescription>,
}

impl BinWrite for DataDescription {
    type Args<'a> = ();

    fn write_options<W: Write + Seek>(
        &self,
        writer: &mut W,
        endian: Endian,
        _: Self::Args<'_>
    ) -> BinResult<()> {
        self.resource_count.write_options(writer, endian, ())?;
        self.padded_size.write_options(writer, endian, ())?;
        self.size.write_options(writer, endian, ())?;
        self.working_buffer_offset.write_options(writer, endian, ())?;
        Ok(())
    }
}

