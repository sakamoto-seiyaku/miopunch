import assert from 'node:assert/strict';
import {chmod, mkdtemp, open, readFile, rename, rm, stat, unlink, writeFile} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import test from 'node:test';
import {sink_abort, sink_save} from '../src/file_storage.js';

function deferred() {
    let resolve;
    const promise = new Promise(done => { resolve = done; });
    return {promise, resolve};
}

// Ordinary owned filesystem capability adapter for the exported sink seam.
// No browser prototypes, production globals or process methods are replaced.
async function storage(afterMove = async () => {}, destinationMode = 0o600) {
    const directory = await mkdtemp(join(tmpdir(), 'miopunch-js-storage-'));
    const temporary = '.partial-file';
    const savedName = 'saved-file';
    let handle = await open(join(directory, temporary), 'wx', 0o600);
    const writerHandle = handle;
    const fallbackWriters = new Set();
    await handle.writeFile('received contents');
    const close = async () => {
        if (handle) await handle.close();
        handle = null;
    };
    const sink = {
        temporary, savedName, saved: false, cancelled: false, saving: null,
        writer: {close, abort: close},
        dir: {
            async getFileHandle(name, options = {}) {
                const path = join(directory, name);
                try { await stat(path); }
                catch (error) {
                    if (error.code !== 'ENOENT') throw error;
                    if (!options.create) throw new DOMException('absent', 'NotFoundError');
                    await (await open(path, 'wx', destinationMode)).close();
                }
                return {
                    async createWritable() {
                        const file = await open(path, 'r+');
                        fallbackWriters.add(file);
                        const close = async () => {
                            await file.close();
                            fallbackWriters.delete(file);
                        };
                        return {
                            write: bytes => file.write(new Uint8Array(bytes)),
                            close, abort: close,
                        };
                    },
                };
            },
            async removeEntry(name) {
                try { await unlink(join(directory, name)); }
                catch (error) {
                    if (error.code === 'ENOENT') throw new DOMException('absent', 'NotFoundError');
                    throw error;
                }
            },
        },
        file: {
            async getFile() {
                return new File([await readFile(join(directory, temporary))], temporary);
            },
            async move(name) {
                await rename(join(directory, temporary), join(directory, name));
                await afterMove();
            },
        },
    };
    return {
        sink,
        writerHandle,
        closeWriter: close,
        path: name => join(directory, name),
        denyRemoval: () => chmod(directory, 0o500),
        async cleanup() {
            await chmod(directory, 0o700);
            await close();
            for (const writer of fallbackWriters) await writer.close();
            await rm(directory, {recursive: true});
        },
    };
}

test('cancellation reports failure when a published file cannot be removed', async () => {
    assert.notEqual(process.getuid(), 0, 'ordinary filesystem permission checks require a non-root test process');
    const moved = deferred();
    const release = deferred();
    const owned = await storage(async () => {
        moved.resolve();
        await release.promise;
    });
    let saving, aborting;
    try {
        saving = sink_save(owned.sink);
        await moved.promise;
        aborting = sink_abort(owned.sink);
        assert.equal(owned.sink.cancelled, true);
        await owned.denyRemoval();
        release.resolve();
        await assert.rejects(aborting, 'cleanup failure must reach the FileSink owner');
        assert.equal(owned.sink.saved, false);
        assert.equal((await stat(owned.path(owned.sink.savedName))).isFile(), true);
    } finally {
        release.resolve();
        await Promise.allSettled([saving, aborting]);
        // This is test-fixture cleanup after the deliberate storage error.
        await owned.cleanup();
    }
});

test('discard retracts a completed JS save still owned by its caller', async () => {
    const owned = await storage();
    try {
        await sink_save(owned.sink);
        assert.equal((await stat(owned.path(owned.sink.savedName))).isFile(), true);
        // Rust clears its sink only after observing the JS save result. A
        // cancellation can win before that observation, while discard still
        // owns this object and no successful receipt has been published.
        await sink_abort(owned.sink);
        await assert.rejects(stat(owned.path(owned.sink.savedName)), {code: 'ENOENT'},
            'a retained sink must retract its unacknowledged output');
    } finally {
        await owned.cleanup();
    }
});

test('writer abort failure is reported after remaining cleanup is attempted', async () => {
    const owned = await storage();
    try {
        await owned.closeWriter();
        // This owned backend is already closed: its normal filesystem
        // operation rejects with EBADF. No process or browser handle is used.
        owned.sink.writer.abort = () => owned.writerHandle.truncate(0);
        await assert.rejects(sink_abort(owned.sink), {code: 'EBADF'},
            'writer abort failure must not be swallowed');
        await assert.rejects(stat(owned.path(owned.sink.temporary)), {code: 'ENOENT'});
    } finally {
        await owned.cleanup();
    }
});

test('fallback writer creation failure still cleans its provisional destination', async () => {
    assert.notEqual(process.getuid(), 0, 'ordinary filesystem permission checks require a non-root test process');
    const owned = await storage(undefined, 0o400);
    delete owned.sink.file.move;
    try {
        await assert.rejects(sink_save(owned.sink), {code: 'EACCES'});
        await assert.rejects(sink_abort(owned.sink), {code: 'EACCES'});
        await assert.rejects(stat(owned.path(owned.sink.savedName)), {code: 'ENOENT'},
            'fallback cleanup must own the destination before opening its writer');
        await assert.rejects(stat(owned.path(owned.sink.temporary)), {code: 'ENOENT'});
    } finally {
        await owned.cleanup();
    }
});

test('fallback saves the actual contents and its retained owner can discard them', async () => {
    const owned = await storage();
    delete owned.sink.file.move;
    try {
        await sink_save(owned.sink);
        assert.equal(await readFile(owned.path(owned.sink.savedName), 'utf8'), 'received contents');
        await assert.rejects(stat(owned.path(owned.sink.temporary)), {code: 'ENOENT'});
        await sink_abort(owned.sink);
        await assert.rejects(stat(owned.path(owned.sink.savedName)), {code: 'ENOENT'});
    } finally {
        await owned.cleanup();
    }
});

test('failed publication preserves an existing destination', async () => {
    const owned = await storage();
    try {
        await writeFile(owned.path(owned.sink.savedName), 'keep existing', {flag: 'wx'});
        await assert.rejects(sink_save(owned.sink), /destination exists/);
        await assert.rejects(sink_abort(owned.sink), /destination exists/);
        assert.equal(await readFile(owned.path(owned.sink.savedName), 'utf8'), 'keep existing');
        await assert.rejects(stat(owned.path(owned.sink.temporary)), {code: 'ENOENT'});
    } finally {
        await owned.cleanup();
    }
});
