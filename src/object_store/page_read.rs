use core::cmp::min;

use crate::file_system::{File, FileSystemError, Offset};
use alloc::{boxed::Box, collections::BTreeMap, vec, vec::Vec};

const PAGE_SIZE: usize = 4096;
const PAGE_SIZE_U64: u64 = 4096;

pub(crate) struct CachingPageReader<F> {
    file: F,
    pages: BTreeMap<Offset, Box<[u8]>>,
}

impl<F: File> CachingPageReader<F> {
    pub fn new(file: F) -> Self {
        Self {
            file,
            pages: BTreeMap::new(),
        }
    }

    async fn get_page(&mut self, page_offset: Offset) -> Result<&[u8], FileSystemError> {
        if !self.pages.contains_key(&page_offset) {
            let mut page = vec![0u8; PAGE_SIZE];
            let read_len = self.file.read_segment(page_offset, &mut page).await?;
            page.truncate(read_len);
            self.pages.insert(page_offset, page.into_boxed_slice());
        }
        let page = self.pages.get(&page_offset).unwrap();
        Ok(page)
    }
}

impl<F: File> File for CachingPageReader<F> {
    async fn read_all(&mut self) -> Result<Vec<u8>, FileSystemError> {
        self.file.read_all().await
    }

    async fn read_segment(
        &mut self,
        offset: Offset,
        dest: &mut [u8],
    ) -> Result<usize, FileSystemError> {
        let mut page_offset = (offset / PAGE_SIZE_U64) * PAGE_SIZE_U64;
        let mut page_start = usize::try_from(offset.0 - page_offset.0).unwrap();
        let mut dest_pos = 0;
        while dest_pos < dest.len() {
            let page = self.get_page(page_offset).await?;
            let page_end = min(page.len(), page_start + dest.len() - dest_pos);
            let dest_end = dest_pos + page_end - page_start;
            dest[dest_pos..dest_end].copy_from_slice(&page[page_start..page_end]);
            dest_pos = dest_end;
            if page.len() < PAGE_SIZE {
                break;
            }
            page_start = 0;
            page_offset = page_offset + PAGE_SIZE_U64;
        }
        Ok(dest_pos)
    }
}

#[cfg(test)]
mod tests {
    use futures::executor::block_on;

    use super::*;
    use std::io::{Cursor, Read, Seek, SeekFrom};

    impl<T: AsRef<[u8]>> File for Cursor<T> {
        async fn read_all(&mut self) -> Result<Vec<u8>, FileSystemError> {
            self.seek(SeekFrom::Start(0)).unwrap();
            let mut out = Vec::new();
            self.read_to_end(&mut out).unwrap();
            Ok(out)
        }

        async fn read_segment(
            &mut self,
            offset: Offset,
            dest: &mut [u8],
        ) -> Result<usize, FileSystemError> {
            let available_len = u64::try_from(self.get_ref().as_ref().len()).unwrap() - offset.0;
            let read_len = min(usize::try_from(available_len).unwrap(), dest.len());
            self.seek(SeekFrom::Start(offset.0)).unwrap();
            self.read_exact(&mut dest[0..(read_len)]).unwrap();
            Ok(read_len)
        }
    }

    #[test]
    fn read_whole_page() {
        let mut buf = Vec::with_capacity(4 * PAGE_SIZE);
        for i in 0..4u8 {
            buf.extend_from_slice(&[i; PAGE_SIZE]);
        }
        let cur = Cursor::new(buf);
        let mut buf = [0u8; PAGE_SIZE];
        let mut reader = CachingPageReader::new(cur);
        block_on(reader.read_segment(Offset(2 * PAGE_SIZE_U64), &mut buf)).unwrap();
        assert!(buf.iter().all(|b| *b == 2));
    }

    #[test]
    fn read_across_page_boundary() {
        let mut buf = Vec::with_capacity(4 * PAGE_SIZE);
        for i in 0..4u8 {
            buf.extend_from_slice(&[i; PAGE_SIZE]);
        }
        let cur = Cursor::new(buf);
        let mut buf = [0u8; PAGE_SIZE];
        let mut reader = CachingPageReader::new(cur);
        block_on(reader.read_segment(Offset(PAGE_SIZE_U64 + PAGE_SIZE_U64 / 2), &mut buf)).unwrap();
        assert!(&buf[0..PAGE_SIZE / 2].iter().all(|b| *b == 1));
        assert!(&buf[PAGE_SIZE / 2..].iter().all(|b| *b == 2));
    }

    #[test]
    fn read_across_many_pages() {
        let mut buf = Vec::with_capacity(4 * PAGE_SIZE);
        for i in 0..4u8 {
            buf.extend_from_slice(&[i; PAGE_SIZE]);
        }
        let cur = Cursor::new(buf);
        let mut buf = [0u8; 2 * PAGE_SIZE];
        let mut reader = CachingPageReader::new(cur);
        block_on(reader.read_segment(Offset(PAGE_SIZE_U64 + PAGE_SIZE_U64 / 2), &mut buf)).unwrap();
        let mut expected = Vec::with_capacity(2 * PAGE_SIZE);
        expected.extend_from_slice(&[1u8; PAGE_SIZE / 2]);
        expected.extend_from_slice(&[2u8; PAGE_SIZE]);
        expected.extend_from_slice(&[3u8; PAGE_SIZE / 2]);
        assert_eq!(buf.as_slice(), &expected);
    }

    #[test]
    fn read_last_segment() {
        let mut buf = Vec::with_capacity(PAGE_SIZE + 1);
        buf.extend_from_slice(&[1u8; PAGE_SIZE]);
        buf.push(2);
        let cur = Cursor::new(buf);
        let mut buf = [0u8; 4];
        let mut reader = CachingPageReader::new(cur);
        let read_len = block_on(reader.read_segment(Offset(PAGE_SIZE_U64 - 2), &mut buf)).unwrap();
        assert_eq!(read_len, 3);
        assert_eq!(buf, [1, 1, 2, 0]);
    }
}
