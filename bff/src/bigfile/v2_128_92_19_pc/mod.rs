pub mod block;
pub mod header;
pub mod resource;

use std::collections::HashMap;
use std::io::{Cursor, Read, Seek, SeekFrom, Write};

use binrw::{BinRead, BinResult, BinWrite, Endian};
use block::*;
use header::{BlockDescription, DataDescription, Header, Resources};
use resource::Resource;

use crate::helpers::{write_align_to, DynArray};
use crate::lz::{lz4_compress_data_writer};
use crate::{bigfile, BffResult};
use crate::bigfile::BigFile;
use crate::bigfile::manifest::*;
use crate::bigfile::platforms::Platform;
use crate::bigfile::versions::Version;
use crate::names::NameType::Asobo64;
use crate::names::{Name, NameType};
use crate::traits::BigFileIo;

use super::versions::VersionXple;

pub struct BigFileV2_128_92_19PC;

#[binrw::parser(reader, endian)]
pub fn blocks_parser(
    block_descriptions: Vec<BlockDescription>,
    resources: &mut HashMap<Name, crate::bigfile::resource::Resource>,
) -> BinResult<Vec<ManifestBlock>> {
    let mut blocks: Vec<ManifestBlock> = Vec::with_capacity(block_descriptions.len() * 2);

    for block_description in block_descriptions {
        // Resources block
        reader.seek(SeekFrom::Start(
            block_description.resources_map_offset as u64 * 2048,
        ))?;
        let block_resource_descriptions = Resources::read_options(reader, endian, ())?;

        let mut first_block_resources = Vec::with_capacity(
            block_resource_descriptions.resources.len()
        );

        for resource in block_resource_descriptions.resources.into_iter() {
            reader.seek(SeekFrom::Start(resource.offset as u64 * 2048))?;
            let resource = Resource::read_options(reader, endian, ())?;

            first_block_resources.push(ManifestResource {
                name: resource.name,
                compress: Some(resource.compress),
            });

            resources.insert(resource.name, resource.into());
        }

        blocks.push(ManifestBlock {
            // offset: Some(block_resource_descriptions.working_buffer_offset as u64), 
            offset: Some(4096), 
            checksum: None,
            compress: None,
            resources: first_block_resources,
        });

        // Data block
        reader.seek(SeekFrom::Start(
            block_resource_descriptions.data_offset as u64 * 2048,
        ))?;

        let mut second_block_resources = Vec::new();
        for data_description in block_resource_descriptions.data_descriptions {
            let data = Data::read_options(reader, endian, (data_description.resource_count,))?;

            for resource in data.resources.into_iter() {
                second_block_resources.push(ManifestResource {
                    name: resource.name,
                    compress: Some(resource.compress),
                });

                resources.insert(resource.name, resource.into());
            }
        }

        blocks.push(ManifestBlock {
            offset: Some(block_resource_descriptions.data_offset as u64 * 2048),
            checksum: None,
            compress: None,
            resources: second_block_resources,
        });
    }

    Ok(blocks)
}

impl BigFileIo for BigFileV2_128_92_19PC {
    fn read<R: Read + Seek>(
        reader: &mut R,
        version: Version,
        platform: Platform,
    ) -> BffResult<BigFile> {
        let endian = platform.into();
        let header = Header::read_options(reader, endian, ())?;

        let mut resources = HashMap::new();

        let blocks = blocks_parser(
            reader,
            endian,
            (header.block_descriptions.inner, &mut resources),
        )?;


        Ok(BigFile {
            manifest: Manifest {
                version,
                version_xple: Some(header.version_oneple.into()),
                platform,
                bigfile_type: Some(header.bigfile_type.into()),
                pool_manifest_unused: None,
                incredi_builder_string: None,
                blocks,
                pool: None,
            },
            resources,
        })
    }

    fn write<W: Write + Seek>(
        bigfile: &BigFile,
        writer: &mut W,
        _tag: Option<&str>,
    ) -> BffResult<()> {
        let endian: Endian = bigfile.manifest.platform.into();
        let begin = writer.stream_position()?;
        
        let mut block_descriptions: Vec<BlockDescription> = 
            Vec::with_capacity(bigfile.manifest.blocks.len());

        let mut total_resource_count = 0u32;
        let mut total_decompressed_size = 0u64;
        let mut total_padded_block_size = 0u64;        

        let mut is_data_block = false; // needed to toggle resource/data,
                                       // because blocks_parser is writing to
                                       // manifest as 2 separate blocks

        let mut data_off = 0;
        let mut res_off = 0;

        let mut data_descriptions = Vec::new();
        let mut datas = Vec::new();
        let mut resmap = Vec::new();

        writer.seek(SeekFrom::Start(4096))?;

        // expected only 2 blocks, so in case of more expected crash
        for block in &bigfile.manifest.blocks {
            let block_beg = writer.stream_position()?;
            let mut res_size = 0;

            for res_ref in &block.resources {
                let resource = bigfile.resources.get(&res_ref.name).unwrap();

                let mut res_writer = Cursor::new(Vec::new());
                Resource::dump_resource(resource, &mut res_writer, endian)?;

                let data = res_writer.into_inner();
                let dec_size = data.len() as u64;
                total_decompressed_size += dec_size;

                let r_offset = writer.stream_position()?;

                let data_to_write = if res_ref.compress.unwrap_or(false){
                    let mut tmp = Cursor::new(Vec::<u8>::new());
                    lz4_compress_data_writer(&data, &mut tmp, endian, ())?;
                    tmp.into_inner()
                }else{data};

                res_size += data_to_write.len();
                total_resource_count += 1;

                if !is_data_block {
                    
                    writer.write_all(&data_to_write)?;
                    write_align_to(writer, 2048, 0)?;

                    resmap.push(bigfile::v2_128_92_19_pc::header::Resource{
                        name: resource.name,
                        class_name: resource.class_name,
                        offset: r_offset as u32 / 2048,
                        size: data_to_write.len() as u64,
                        decompressed_size: dec_size,
                    });
                }else{
                    datas.push(data_to_write);
                }
            }

            let padded_block_size = res_size.div_ceil(2048) * 2048;
            if is_data_block {
                data_off = block_beg / 2048;

                data_descriptions.push(DataDescription{
                    resource_count: block.resources.len() as u32,
                    padded_size: padded_block_size as u64,
                    size: res_size as u64,
                    working_buffer_offset: block_beg // unknown
                });

                total_padded_block_size += padded_block_size as u64;
            } else {
                res_off = block_beg;
                let end_of_data_pos = writer.stream_position()?;
                let res_map_off = (end_of_data_pos / 2048) as u32;

                block_descriptions.push(BlockDescription {
                    unk1: 0,
                    unk2: 0,
                    unk3: 0,
                    resources_map_offset: res_map_off + 1,
                    data_resources_map_offset: data_off as u32 + 1
                });
            }

            is_data_block = !is_data_block;
        }

        let end_of_data_pos = writer.stream_position()?;

        writer.seek(SeekFrom::Start(end_of_data_pos))?;
        let block_description_offset = (end_of_data_pos / 2048) as u32;
       
        let blocks_count = block_descriptions.len() as u32;

        // at least i think is a block_description count
        writer.write_all(&blocks_count.to_le_bytes())?;

        for desc in &block_descriptions{
            desc.write_options(writer, endian, ())?;
            write_align_to(writer, 2048, 0)?;
        }

        let padding = (2048 - (end_of_data_pos % 2048)) % 2048;
        let resources = Resources{
            data_offset: block_description_offset + 2,
            working_buffer_offset: res_off as u32, // ???????? seems it's not a buffer offest
                                                   // for example bytes is 00 10 A0 00 (10489856)
                                                   // which is bigger than file size
            unk1: 0,
            unk2: 0,
            padded_size: data_descriptions.iter().map(|d| d.padded_size).sum(),
            padding_size: padding,
            data_descriptions,
            resources: resmap,
            unk3: 0
        };

        // manually writing Resources struct, 
        // because i don't know how implement BinWrite

        let data_count = resources.data_descriptions.len() as u32;
        writer.write_all(&data_count.to_le_bytes())?;
        writer.write_all(&resources.data_offset.to_le_bytes())?;
        writer.write_all(&resources.working_buffer_offset.to_le_bytes())?;
        writer.write_all(&resources.unk1.to_le_bytes())?;
        writer.write_all(&resources.unk2.to_le_bytes())?;
        writer.write_all(&resources.padded_size.to_le_bytes())?;
        writer.write_all(&resources.padding_size.to_le_bytes())?;
        for dd in &resources.data_descriptions {
            writer.write_all(&dd.resource_count.to_le_bytes())?;
            writer.write_all(&dd.padded_size.to_le_bytes())?;
            writer.write_all(&dd.size.to_le_bytes())?;
            writer.write_all(&dd.working_buffer_offset.to_le_bytes())?;
        }

        // pad_after = DataDescription::SIZE * 52 - DataDescription::SIZE * data_count as u64
        let pad_after = 28 * 52 - 28 * data_count as usize;
        if pad_after > 0 {
            writer.write_all(&vec![0u8; pad_after])?;
        }

        let resource_count = resources.resources.len() as u32;
        writer.write_all(&resource_count.to_le_bytes())?;

        for res in &resources.resources {
            res.name.write_options(writer, endian, ())?;
            res.class_name.write_options(writer, endian, ())?;
            writer.write_all(&res.offset.to_le_bytes())?;
            writer.write_all(&res.size.to_le_bytes())?;
            writer.write_all(&res.decompressed_size.to_le_bytes())?;
        }

        writer.write_all(&resources.unk3.to_le_bytes())?;
        write_align_to(writer, 2048, 0)?;

        // writing data resources
        for d in datas {
            writer.write_all(&d)?;
        }
    
        write_align_to(writer, 2048, 0)?;

        let end = writer.stream_position()?;

        writer.seek(SeekFrom::Start(end))?;
        let file_size = end;

        writer.seek(SeekFrom::Start(begin))?;

        let header = Header{
            version_oneple: match bigfile.manifest.version_xple.unwrap_or(0.into()) {
                VersionXple::Oneple(x) | VersionXple::Triple((x, _, _)) => x,
            },
            bigfile_type: bigfile
                .manifest
                .bigfile_type
                .unwrap_or(BigFileType::Normal)
                .into(),
            block_description_offset,
            unk1: 0,
            pool_offset: 0, // stock file zero
            unk3: 0,
            unk4: 0,
            unk5: 0,
            total_padded_block_size,
            block_sector_padding_size: 0,
            file_size,
            total_decompressed_size,
            zero: 0,
            total_resource_count,
            block_descriptions: DynArray::from(Vec::new()), // just a dummy, manually writed before
        };

        header.write_options(writer, endian, ())?;

        writer.seek(SeekFrom::Start(end_of_data_pos))?;

        Ok(())
    }

    const NAME_TYPE: NameType = Asobo64;

    type ResourceType = Resource;
}
