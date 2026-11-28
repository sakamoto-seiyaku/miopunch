//! Complete original signed management record, shared by platform stores.
use std::io;

/// Base64 keeps signed byte strings compact inside the browser's JSON snapshot.
pub(crate) mod stored_records {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use serde::{
        Deserialize, Deserializer, Serialize, Serializer,
        de::{Error, Visitor},
    };
    use std::fmt;

    type Record = (Vec<u8>, Option<Vec<u8>>);

    pub(crate) fn serialize<S: Serializer>(
        records: &[Record],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(records.iter().map(|(header, body)| {
            (
                STANDARD.encode(header),
                body.as_ref().map(|body| STANDARD.encode(body)),
            )
        }))
    }

    struct StoredBytes(Vec<u8>);

    impl<'de> Deserialize<'de> for StoredBytes {
        fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
            struct Bytes;
            impl<'de> Visitor<'de> for Bytes {
                type Value = StoredBytes;

                fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    f.write_str("a base64 string")
                }
                fn visit_str<E: Error>(self, value: &str) -> Result<Self::Value, E> {
                    STANDARD.decode(value).map(StoredBytes).map_err(E::custom)
                }
            }
            de.deserialize_str(Bytes)
        }
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(de: D) -> Result<Vec<Record>, D::Error> {
        let records = Vec::<(StoredBytes, Option<StoredBytes>)>::deserialize(de)?;
        Ok(records
            .into_iter()
            .map(|(h, b)| (h.0, b.map(|b| b.0)))
            .collect())
    }

    /// Exact serialized tuple size with padded base64, quotes and delimiters.
    pub(crate) fn json_size((header, body): &Record) -> usize {
        let string_len = |n: usize| 4 * n.div_ceil(3) + 2;
        3 + string_len(header.len()) + body.as_ref().map_or(4, |b| string_len(b.len()))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[derive(Serialize, Deserialize)]
        struct Records(#[serde(with = "super")] Vec<Record>);

        #[test]
        fn compact_storage_preserves_signed_bytes_and_optional_bodies() {
            let records = vec![(vec![0, 127, 255], None), (vec![9], Some(vec![]))];
            let compact = serde_json::to_string(&Records(records.clone())).unwrap();
            assert_eq!(
                compact.len(),
                3 + records.iter().map(json_size).sum::<usize>()
            );
            assert_eq!(
                serde_json::from_str::<Records>(&compact).unwrap().0,
                records
            );
        }
    }
}
use std::sync::Arc;

use p2panda_core::{Hash, Header};

use crate::management_auth::ManagementExtension;

pub(crate) const MAX_PAGE_RECORDS: usize = 4096;
pub(crate) const MAX_PAGE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct ManagementRecord {
    pub(crate) header: Vec<u8>,
    pub(crate) body: Option<Vec<u8>>,
    verified_header: Option<Arc<Header<ManagementExtension>>>,
}

// The local projection cache does not change the original record's identity.
impl PartialEq for ManagementRecord {
    fn eq(&self, other: &Self) -> bool {
        self.header == other.header && self.body == other.body
    }
}
impl Eq for ManagementRecord {}

impl ManagementRecord {
    pub fn new(header: Vec<u8>, body: Option<&[u8]>) -> Self {
        Self::from_parts(header, body.map(Vec::from))
    }

    pub(crate) fn from_parts(header: Vec<u8>, body: Option<Vec<u8>>) -> Self {
        Self {
            header,
            body,
            verified_header: None,
        }
    }

    pub(crate) fn with_verified_header(
        mut self,
        header: Arc<Header<ManagementExtension>>,
    ) -> Option<Self> {
        if !self.matches_header(&header) {
            return None;
        }
        self.verified_header = Some(header);
        Some(self)
    }

    pub(crate) fn verified_header(&self) -> Option<&Arc<Header<ManagementExtension>>> {
        self.verified_header
            .as_ref()
            .filter(|header| self.matches_header(header))
    }

    fn matches_header(&self, header: &Header<ManagementExtension>) -> bool {
        Hash::digest(&self.header) == header.hash()
    }

    /// LogSync already checked the signature. The receiving store must still
    /// validate the body and apply causal and authorization rules.
    pub(crate) fn from_verified_operation(
        header: Header<ManagementExtension>,
        body: Option<Vec<u8>>,
    ) -> Self {
        Self {
            header: header.encode(),
            body,
            verified_header: Some(Arc::new(header)),
        }
    }

    pub(crate) fn verify_complete(
        &self,
        anchor: crate::management_auth::NetworkAnchor,
    ) -> Result<Header<ManagementExtension>, crate::management_auth::AuthError> {
        match self.verified_header() {
            Some(header) => crate::management_auth::validate_complete_header(
                header.as_ref().clone(),
                self.body(),
                anchor,
            ),
            None => crate::management_auth::decode_complete(self.header(), self.body(), anchor),
        }
    }

    pub(crate) fn into_parts(self) -> (Vec<u8>, Option<Vec<u8>>) {
        (self.header, self.body)
    }

    /// Keep the signing builder's typed output in explicit source fixtures.
    /// Network receipt still enters through complete-record verification.
    #[cfg(feature = "replica-test-hooks")]
    #[doc(hidden)]
    pub fn from_signed_header_for_test(
        header: Header<ManagementExtension>,
        body: Option<&[u8]>,
    ) -> Self {
        Self {
            header: header.encode(),
            body: body.map(Vec::from),
            verified_header: Some(Arc::new(header)),
        }
    }

    pub fn header(&self) -> &[u8] {
        &self.header
    }

    pub fn body(&self) -> Option<&[u8]> {
        self.body.as_deref()
    }
}

pub(crate) trait HistoryRecord {
    fn byte_len(&self) -> usize;
}

impl HistoryRecord for ManagementRecord {
    fn byte_len(&self) -> usize {
        (self.header(), self.body()).byte_len()
    }
}

impl<B: AsRef<[u8]>> HistoryRecord for (B, Option<B>) {
    fn byte_len(&self) -> usize {
        self.0.as_ref().len() + self.1.as_ref().map_or(0, |b| b.as_ref().len())
    }
}

/// Take one contiguous page without splitting an original signed record.
pub(crate) fn history_page<T: HistoryRecord>(remaining: &[T]) -> io::Result<&[T]> {
    let mut available = MAX_PAGE_BYTES;
    let mut count = 0;
    for record in remaining.iter().take(MAX_PAGE_RECORDS) {
        let Some(left) = available.checked_sub(record.byte_len()) else {
            break;
        };
        available = left;
        count += 1;
    }
    if count == 0 && !remaining.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "management record exceeds page limit",
        ));
    }
    Ok(&remaining[..count])
}

/// Include the protocol-specific envelope and actual encoding in the frame budget.
pub(crate) fn encode_history_page<T: HistoryRecord>(
    remaining: &[T],
    mut encode: impl FnMut(&[T]) -> io::Result<Vec<u8>>,
) -> io::Result<(usize, Vec<u8>)> {
    let mut page = history_page(remaining)?;
    loop {
        let encoded = encode(page)?;
        if encoded.len() <= MAX_PAGE_BYTES {
            return Ok((page.len(), encoded));
        }
        if page.len() <= 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "encoded management frame exceeds page limit",
            ));
        }
        page = &page[..page.len().div_ceil(2)];
    }
}

/// Binary page of complete original records, preserving None and an empty body.
pub(crate) fn encode_raw_records(records: &[(Vec<u8>, Option<Vec<u8>>)]) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    if records.len() > MAX_PAGE_RECORDS {
        return Err(raw_page_error());
    }
    bytes.extend_from_slice(&(records.len() as u32).to_be_bytes());
    for (header, body) in records {
        for value in [Some(header), body.as_ref()] {
            match value {
                Some(value) => {
                    let length = u32::try_from(value.len()).map_err(|_| raw_page_error())?;
                    if length == u32::MAX || bytes.len() + 4 + value.len() > MAX_PAGE_BYTES {
                        return Err(raw_page_error());
                    }
                    bytes.extend_from_slice(&length.to_be_bytes());
                    bytes.extend_from_slice(value);
                }
                None => bytes.extend_from_slice(&u32::MAX.to_be_bytes()),
            }
        }
    }
    if bytes.len() > MAX_PAGE_BYTES {
        return Err(raw_page_error());
    }
    Ok(bytes)
}

pub(crate) fn decode_raw_records(mut bytes: &[u8]) -> io::Result<Vec<(Vec<u8>, Option<Vec<u8>>)>> {
    fn length(bytes: &mut &[u8]) -> io::Result<u32> {
        let raw = bytes.get(..4).ok_or_else(raw_page_error)?;
        let value = u32::from_be_bytes(raw.try_into().unwrap());
        *bytes = &bytes[4..];
        Ok(value)
    }
    fn field(bytes: &mut &[u8], length: u32) -> io::Result<Vec<u8>> {
        let value = bytes
            .get(..length as usize)
            .ok_or_else(raw_page_error)?
            .to_vec();
        *bytes = &bytes[length as usize..];
        Ok(value)
    }
    if bytes.len() > MAX_PAGE_BYTES {
        return Err(raw_page_error());
    }
    let count = length(&mut bytes)? as usize;
    if count > MAX_PAGE_RECORDS {
        return Err(raw_page_error());
    }
    let mut records = Vec::new();
    for _ in 0..count {
        let size = length(&mut bytes)?;
        let header = field(&mut bytes, size)?;
        let size = length(&mut bytes)?;
        let body = if size == u32::MAX {
            None
        } else {
            Some(field(&mut bytes, size)?)
        };
        records.push((header, body));
    }
    if !bytes.is_empty() {
        return Err(raw_page_error());
    }
    Ok(records)
}

fn raw_page_error() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid raw management page")
}

#[cfg(test)]
mod raw_tests {
    use super::*;
    #[test]
    fn raw_records_preserve_bytes_and_optional_empty_bodies() {
        let bytes: Vec<u8> = (0..=255).collect();
        let records = vec![
            (bytes.clone(), Some(bytes)),
            (vec![7], None),
            (vec![9], Some(vec![])),
        ];
        let raw = encode_raw_records(&records).unwrap();
        assert_eq!(raw.len(), 4 + 3 * 8 + 514);
        assert_eq!(decode_raw_records(&raw).unwrap(), records);
        for end in 0..raw.len() {
            assert!(decode_raw_records(&raw[..end]).is_err());
        }
        let mut extra = raw;
        extra.push(0);
        assert!(decode_raw_records(&extra).is_err());
    }
}

/// Exact raw-page size, including the calling protocol's binary envelope.
pub(crate) fn raw_records_page(
    records: &[(Vec<u8>, Option<Vec<u8>>)],
    envelope_bytes: usize,
) -> io::Result<(usize, Vec<u8>)> {
    let mut size = 4usize
        .checked_add(envelope_bytes)
        .ok_or_else(raw_page_error)?;
    let mut count = 0;
    for record in records.iter().take(MAX_PAGE_RECORDS) {
        let next = size
            .checked_add(8 + record.byte_len())
            .ok_or_else(raw_page_error)?;
        if next > MAX_PAGE_BYTES {
            break;
        }
        size = next;
        count += 1;
    }
    if count == 0 && !records.is_empty() {
        return Err(raw_page_error());
    }
    Ok((count, encode_raw_records(&records[..count])?))
}
