#![allow(non_snake_case)]
#![feature(seek_convenience)]
use std::path::Path;
use std::path::PathBuf;
use std::{
    ffi::OsString,
    io::{BufReader, BufWriter, SeekFrom},
};

use io::{Error, ErrorKind};
use std::fs::{self, DirEntry, File, OpenOptions};
use std::io;
use std::io::Read;
use std::process::exit;
use std::{convert::TryInto, io::prelude::*};
use structopt::StructOpt;

const PAGE_LENGTH: u16 = 0x4000;
const BLOCK_SIZE: u16 = 0x100;
const KFS_FILE_ID: u8 = 0x7F;
const KFS_DIR_ID: u8 = 0xBF;
const KFS_SYM_ID: u8 = 0xDF;
const KFS_VERSION: u8 = 0x0;

#[derive(Debug, StructOpt)]
#[structopt(name = "regenkfs")]
/// A reimplementation of the KnightOS genkfs tool in Rust.
///
struct Opt {
    /// The ROM file to write the filesystem to.
    #[structopt(parse(from_os_str))]
    input: PathBuf,

    /// Path to a directory that will be copied into / on the new filesystem.
    model: PathBuf,
}

struct Context<'a> {
    rom_path: PathBuf,
    model: &'a Path,
    fat_start: u8,
    dat_start: u8,
    rom: BufWriter<File>,
}

fn div_rem<T: std::ops::Div<Output = T> + std::ops::Rem<Output = T> + Copy>(x: T, y: T) -> (T, T) {
    let quot = x / y;
    let rem = x % y;
    (quot, rem)
}

impl<'a> Context<'a> {
    fn new(rom_path: &'a Path, model: &'a Path) -> Result<Context<'a>, Error> {
        if !model.is_dir() {
            eprintln!("Unable to open {}.", model.display());
            exit(1);
        }
        if !rom_path.is_file() {
            Error::new(
                ErrorKind::NotFound,
                format!("Unable to open {}.", rom_path.display()),
            );
        }

        let length = fs::metadata(&rom_path)?.len();
        // This opens the file like fopen(rom_file, "r+") in C.
        let rom = BufWriter::new(
            OpenOptions::new()
                .write(true)
                .truncate(false)
                .open(&rom_path)?,
        );

        let fat_start: u8 = if cfg!(feature = "c-undef") {
            // C original has undefined behavior: context.fat_start = length / PAGE_LENGTH - 0x9;
            TryInto::<u8>::try_into(length / u64::from(PAGE_LENGTH))
                .unwrap()
                .wrapping_sub(9)
        } else {
            // Safe version
            TryInto::<u8>::try_into(length / u64::from(PAGE_LENGTH) - 9)
                .map_err(|err| Error::new(ErrorKind::InvalidData, err))?
        };
        Ok(Context {
            rom_path: rom_path.to_path_buf(),
            model,
            fat_start,
            dat_start: 0x04,
            rom,
        })
    }

    fn write_fat(&mut self, entry: Vec<u8>, length: u16, fatptr: &mut u32) -> Result<(), Error> {
        *fatptr -= u32::from(length);
        self.rom.seek(SeekFrom::Start(u64::from(*fatptr)))?;
        self.rom.write_all(&entry[..usize::from(length)])?;
        self.rom.flush()
    }

    fn write_block(&mut self, file: &mut BufReader<File>, section_id: u16) -> Result<(), Error> {
        let [l, h] = section_id.to_le_bytes();
        let flash_page: u16 = u16::from(h);
        let index: u16 = u16::from(l);
        self.rom.seek(SeekFrom::Start(
            u64::from(flash_page) * u64::from(PAGE_LENGTH)
                + u64::from(index) * u64::from(BLOCK_SIZE),
        ))?;
        let mut block: [u8; BLOCK_SIZE as usize] = [0x0; BLOCK_SIZE as usize];
        let len = file.read(&mut block)?;
        self.rom.write_all(&block[..len])?;
        self.rom.flush()
    }

    fn write_dat(
        &mut self,
        file: &mut BufReader<File>,
        length: u32,
        section_id: &mut u16,
    ) -> Result<(), Error> {
        let mut length = length;
        let mut pSID: u16 = 0xFFFF;
        file.seek(SeekFrom::Start(0))?;
        while length > 0 {
            /* Prep */
            let [l, h] = (*section_id).to_le_bytes();
            let mut flash_page: u16 = u16::from(h);
            let mut index: u8 = l;
            let mut nSID: u16 = 0xFFFF;
            let header_addr: u32 =
                u32::from(PAGE_LENGTH) * u32::from(flash_page) + u32::from(index) * 4;
            index += 1;
            if index > 0x3F {
                index = 1;
                flash_page += 1;
                /* Write the magic number */
                self.rom.seek(SeekFrom::Start(
                    u64::from(flash_page) * u64::from(PAGE_LENGTH),
                ))?;
                self.rom.write_all(b"KFS")?;
                self.rom.write_all(&[0xFF << KFS_VERSION])?;
            }
            if length > u32::from(BLOCK_SIZE) {
                nSID = (flash_page << 8) | u16::from(index);
            }

            /* Section header */
            self.rom.seek(SeekFrom::Start(u64::from(header_addr)))?;

            pSID &= 0x7FFF; // Mark this section in use

            // Warning: original C code uses fwrite which is
            // arch-dependent.  We choose little endian here.
            self.rom.write_all(&pSID.to_le_bytes())?;
            self.rom.write_all(&nSID.to_le_bytes())?;

            /* Block data */
            self.write_block(file, *section_id)?;
            self.rom.flush()?;

            length = length.saturating_sub(u32::from(BLOCK_SIZE));
            pSID = *section_id;
            *section_id = (flash_page << 8) | u16::from(index);
        }
        Ok(())
    }

    fn write_recursive(
        &mut self,
        model: PathBuf,
        parent_id: &mut u16,
        section_id: &mut u16,
        fatptr: &mut u32,
    ) -> Result<(), Error> {
        let parent: u16 = *parent_id;
        // Put paths into a Vec to sort alphabetically.
        let mut paths: Vec<DirEntry> = fs::read_dir(model)?.map(|r| r.unwrap()).collect();
        paths.sort_by_key(|dir| dir.path());
        for entry in paths {
            let path = entry.path();
            if entry.file_type()?.is_symlink() {
                let target = path.read_link().expect("Failed to follow symlink");
                println!(
                    "Adding link from {} to {}...",
                    path.display(),
                    target.display()
                );

                let entry_name: OsString = entry.file_name();
                let entry_name_bytes: &[u8] = entry_name.to_str().unwrap().as_bytes();

                // Use .to_str() instead of .file_name() to avoid
                // losing relative path.
                // (i.e. want ../foo.c instead of foo.c)
                let target_name: &str = target.to_str().unwrap();
                let target_name_bytes: &[u8] = target_name.as_bytes();

                let dl: u16 = entry_name.len().try_into().unwrap();
                let tl: u16 = target_name_bytes.len().try_into().unwrap();

                let elen: u16 = dl + tl + 5;
                let mut sentry: Vec<u8> = vec![0x0; usize::from(elen) + 3];

                sentry[0] = KFS_SYM_ID;
                sentry[1..=2].clone_from_slice(&elen.to_le_bytes());
                sentry[3..=4].clone_from_slice(&parent.to_le_bytes());
                sentry[5] = (dl + 1).try_into().unwrap();
                sentry[6..][..usize::from(dl)].clone_from_slice(entry_name_bytes);
                sentry[usize::from(7 + dl)..][..usize::from(tl)]
                    .clone_from_slice(target_name_bytes);
                sentry.reverse();
                self.write_fat(sentry, elen + 3, fatptr)?
            } else if path.is_dir() {
                let entry_name: OsString = entry.file_name();
                let entry_str = entry_name.to_str().unwrap();
                let entry_name_bytes: &[u8] = entry_str.as_bytes();
                let elen: u16 = (entry_name.len() + 6).try_into().map_err(|_| {
                    Error::new(
                        ErrorKind::InvalidData,
                        format!("Filename too long: {}", entry_str),
                    )
                })?;

                let mut fentry: Vec<u8> = vec![0x0; usize::from(elen) + 3];
                println!("Adding {}...", path.display());
                fentry[0] = KFS_DIR_ID;
                fentry[1..=2].clone_from_slice(&elen.to_le_bytes());
                fentry[3..=4].clone_from_slice(&parent.to_le_bytes());
                *parent_id += 1;
                fentry[5..=6].clone_from_slice(&(*parent_id).to_le_bytes());
                fentry[7] = 0xFF; // Flags
                fentry[8..][..entry.file_name().len()].clone_from_slice(entry_name_bytes);
                fentry.reverse();
                self.write_fat(fentry, elen + 3, fatptr)?;
                self.write_recursive(path, parent_id, section_id, fatptr)?
            } else if path.is_file() {
                let entry_name: OsString = entry.file_name();
                let entry_str = entry_name.to_str().unwrap();
                let entry_name_bytes: &[u8] = entry_str.as_bytes();
                let elen: u16 = (entry_name.len() + 9).try_into().map_err(|_| {
                    Error::new(
                        ErrorKind::InvalidData,
                        format!("Filename too long: {}", entry_str),
                    )
                })?;
                let len = path.metadata()?.len();
                if len > 0xFFFFFF {
                    eprintln!(
                        "Error: {} is larger than the maximum file size.",
                        path.display()
                    );

                    exit(1);
                }
                println!("Adding {}...", path.display());
                // Now safe to coerce len into u32
                let len: u32 = len.try_into().unwrap();
                let mut fentry: Vec<u8> = vec![0x0; usize::from(elen) + 3];

                fentry[0] = KFS_FILE_ID;
                fentry[1..=2].clone_from_slice(&elen.to_le_bytes());
                fentry[3..=4].clone_from_slice(&parent.to_le_bytes());
                fentry[5] = 0xFF; // Flags
                fentry[6..=8].clone_from_slice(&len.to_le_bytes()[0..=2]);
                fentry[9] = (*section_id).to_le_bytes()[0];
                fentry[10] = (*section_id).to_le_bytes()[1];
                fentry[11..][..entry.file_name().len()].clone_from_slice(entry_name_bytes);
                fentry.reverse();
                self.write_fat(fentry, elen + 3, fatptr)?;
                self.write_dat(&mut BufReader::new(File::open(path)?), len, section_id)?
            } else {
                unreachable!();
            }
        }
        Ok(())
    }

    // Returns the number of data pages (low byte) and fat pages (high
    // byte) written.
    fn write_filesystem(&mut self) -> Result<u16, Error> {
        let mut parent_id: u16 = 0;
        let mut section_id: u16 = ((u16::from(self.dat_start)) << 8) | 1;
        let mut fatptr: u32 = (u32::from(self.fat_start) + 1) * u32::from(PAGE_LENGTH);
        let fatptr_start: u32 = fatptr;
        /* Write the first DAT page's magic number */
        self.rom.seek(SeekFrom::Start(
            u64::from(self.dat_start) * u64::from(PAGE_LENGTH),
        ))?;
        self.rom.write_all(b"KFS")?;
        self.rom.flush()?;
        self.write_recursive(
            self.model.to_path_buf(),
            &mut parent_id,
            &mut section_id,
            &mut fatptr,
        )?;

        let (quot, rem) = div_rem(fatptr_start - fatptr, u32::from(PAGE_LENGTH));
        // Given that PAGE_LENGTH is sufficiently large, it's safe to
        // downgrade number size here.
        let mut result: u16 = quot.try_into().unwrap();
        if rem > 0 {
            result += 1;
        }
        result <<= 8;
        // sectionId's high byte is a page number
        result = if cfg!(feature = "c-undef") {
            // C original has undefined behavior:  result += (sectionId >> 8) - dat_start + 1;
            result.wrapping_add((section_id >> 8).wrapping_sub(u16::from(self.dat_start))) + 1
        } else {
            // Safe version
            result + (section_id >> 8) - u16::from(self.dat_start) + 1
        };
        Ok(result)
    }
    fn run(&mut self) -> Result<(), Error> {
        let mut blank_page: [u8; PAGE_LENGTH as usize] = [0xFF; PAGE_LENGTH as usize];
        self.rom.seek(SeekFrom::Start(
            u64::from(self.dat_start) * u64::from(PAGE_LENGTH),
        ))?;
        for p in self.dat_start..(self.fat_start + 1) {
            blank_page[0] = if p <= self.fat_start - 4 { b'K' } else { 0xFF };
            self.rom.write_all(&blank_page)?;
        }
        self.rom.flush()?;

        let result = self.write_filesystem();
        self.rom.flush()?;
        println!(
            "Filesystem successfully written to {}.",
            self.rom_path.display()
        );
        print!("Indexes of written data pages: ");
        let [lo, hi] = result?.to_le_bytes();
        for i in 0..u32::from(lo) {
            print!("{:02x} ", u32::from(self.dat_start) + i)
        }
        print!("\nIndexes of written FAT pages: ");
        for i in 0..u32::from(hi) {
            print!("{:02x} ", u32::from(self.fat_start) - i)
        }
        println!("\nThe rest of the pages (except kernels' 00-03) are empty.");
        Ok(())
    }
}

fn main() {
    let opt: Opt = Opt::from_args();
    match Context::new(&opt.input, &opt.model).and_then(|mut c| c.run()) {
        Ok(()) => exit(0),
        Err(_) => exit(1),
    }
}
