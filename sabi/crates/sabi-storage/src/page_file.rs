/*
let page_id = page_file.allocate_page()?;  // e.g., returns 0

// 3. Create and populate a page
let mut page = Page::new(page_id, PageType::Data);
page.payload = b"Hello, World!".to_vec();
page.header.payload_len = page.payload.len() as u32;

// 4. Write page to disk
page_file.write_page(&page)?;

// 5. Read it back
let loaded_page = page_file.read_page(page_id)?;
assert_eq!(loaded_page.payload, b"Hello, World!");

// 6. Free the page (for reuse)
page_file.free_page(page_id);

*/

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use sabi_core::error::{Result};

use crate::page::{Page, PAGE_SIZE};

pub struct PageFile {
    file: File, // OS file handle
    free_list: Vec<u64>, // Reusable page IDs
}

impl PageFile {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let file = OpenOptions::new()
            .read(true) // Can read
            .write(true) // Can write
            .create(true)  // Create if doesn't exist
            .open(path)?;

        Ok(Self {
            file,
            free_list: Vec::new(),
        })
    }

    pub fn read_page(&mut self, page_id: u64) -> Result<Page> {
        // calculate starting position for page
        let offset = page_id * PAGE_SIZE as u64;

        // move cursor to page start
        self.file.seek(SeekFrom::Start(offset))?;

        // Buffer for raw page
        let mut buf = [0u8; PAGE_SIZE];

        // Read exactly 4096 bytes
        self.file.read_exact(&mut buf)?;

        Page::deserialize(&buf)
    }

    pub fn write_page(&mut self, page: &Page) -> Result<()> {
        // calculate starting position for page
        let offset = page.header.page_id * PAGE_SIZE as u64;

        // move cursor to page start
        self.file.seek(SeekFrom::Start(offset))?;

        let buf = page.serialize()?;
        // Write to file
        self.file.write_all(&buf)?;
        // Flush to disk
        self.file.sync_data()?;

        Ok(())
    }

    pub fn allocate_page(&mut self) -> Result<u64> {
        // try checking for freed pages to reuse
        if let Some(id) = self.free_list.pop() {
            Ok(id)
        } else {
            // Otherwise, allocate at end of file
            let len = self.file.metadata()?.len();
            // calculate number of pages (id of new page)
            Ok(len / PAGE_SIZE as u64)
        }
    }

    pub fn free_page(&mut self, page_id: u64) {
        self.free_list.push(page_id);
    }
}
