// Browser file capability adapter. Blob slicing and OPFS writes are awaited one
// bounded chunk at a time; no file-sized array or persistent task queue exists.
export function file_metadata(file) {
    if (!(file instanceof Blob) || !Number.isSafeInteger(file.size) || file.size < 0
        || typeof file.name !== 'string') throw new Error('invalid file source');
    return JSON.stringify({name: file.name, size: file.size});
}

export async function file_slice(file, offset, length) {
    if (!Number.isSafeInteger(offset) || !Number.isInteger(length) || length < 0 || length > 65536)
        throw new Error('invalid file read');
    return new Uint8Array(await file.slice(offset, offset + length).arrayBuffer());
}

async function directory(namespace) {
    if (!/^[a-f0-9]{64}$/.test(namespace)) throw new Error('invalid file namespace');
    const root = await navigator.storage.getDirectory();
    return root.getDirectoryHandle('miopunch-files-' + namespace, {create: true});
}

export async function files_enable(namespace) {
    await directory(namespace);
}

async function absent(dir, name) {
    try { await dir.getFileHandle(name); }
    catch (error) { if (error.name === 'NotFoundError') return; throw error; }
    throw new Error('destination exists');
}

export async function sink_open(namespace, savedName, id) {
    const dir = await directory(namespace);
    const temporary = '.partial-' + id;
    await absent(dir, temporary);
    await absent(dir, savedName);
    const file = await dir.getFileHandle(temporary, {create: true});
    try {
        return {dir, file, temporary, savedName, destinationOwned: false,
            writer: await file.createWritable({keepExistingData: false}), saved: false, cancelled: false, saving: null};
    } catch (error) {
        await dir.removeEntry(temporary);
        throw error;
    }
}

export async function sink_write(sink, offset, bytes) {
    if (!(bytes instanceof Uint8Array) || bytes.byteLength > 65536)
        throw new Error('invalid file chunk');
    await sink.writer.write({type: 'write', position: offset, data: bytes});
}

function cancellation(sink) {
    return sink.cancellationError ??= new DOMException('file cancelled', 'AbortError');
}

async function commit(sink) {
    await sink.writer.close();
    sink.writer = null;
    if (sink.cancelled) throw cancellation(sink);
    await absent(sink.dir, sink.savedName);
    if (sink.cancelled) throw cancellation(sink);
    sink.destinationOwned = true;
    if (typeof sink.file.move === 'function') {
        await sink.file.move(sink.savedName);
    } else {
        // The fallback is also bounded and closes the actual destination before
        // success. It is not an in-memory download or a simulated save.
        const source = await sink.file.getFile();
        const destination = await sink.dir.getFileHandle(sink.savedName, {create: true});
        sink.writer = await destination.createWritable();
        for (let offset = 0; offset < source.size; offset += 65536) {
            if (sink.cancelled) throw cancellation(sink);
            await sink.writer.write(await source.slice(offset, offset + 65536).arrayBuffer());
        }
        await sink.writer.close();
        sink.writer = null;
        await sink.dir.removeEntry(sink.temporary);
    }
    // A filesystem close/move Promise can complete after its Rust future was
    // cancelled. Its owner still observes the cancellation before publishing
    // success. Discard owns both temporary and provisional output cleanup.
    if (sink.cancelled) throw cancellation(sink);
    sink.saved = true;
}

export function sink_save(sink) {
    sink.saving ??= commit(sink);
    return sink.saving;
}

export async function sink_abort(sink) {
    // Rust relinquishes the object after observing save success. If it still
    // calls discard, even a resolved JS save remains unacknowledged and owned.
    sink.cancelled = true;
    const failures = [];
    if (sink.saving) {
        try { await sink.saving; }
        catch (error) {
            if (!sink.cancellationError || error !== sink.cancellationError) failures.push(error);
        }
    }
    if (sink.writer) {
        try { await sink.writer.abort(); }
        catch (error) { failures.push(error); }
    }
    sink.writer = null;
    if (sink.destinationOwned) {
        try {
            await sink.dir.removeEntry(sink.savedName);
            sink.destinationOwned = false;
            sink.saved = false;
        } catch (error) {
            if (error?.name === 'NotFoundError') {
                sink.destinationOwned = false;
                sink.saved = false;
            } else failures.push(error);
        }
    }
    try { await sink.dir.removeEntry(sink.temporary); }
    catch (error) { if (error?.name !== 'NotFoundError') failures.push(error); }
    if (failures.length) throw failures[0];
}

export async function received_file(namespace, name) {
    if (typeof name !== 'string' || !/^[a-f0-9]{32}-/.test(name) || /[\\/]/.test(name))
        throw new Error('invalid saved file name');
    const dir = await directory(namespace);
    return (await dir.getFileHandle(name)).getFile();
}
export function file_capable() {
  return typeof File === 'function' && typeof Blob.prototype.slice === 'function'
    && typeof navigator.storage?.getDirectory === 'function';
}
