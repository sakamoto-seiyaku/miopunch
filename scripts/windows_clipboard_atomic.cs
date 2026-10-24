// Independent interactive validation host. No clipboard value/digest leaves
// this object. All native clipboard transactions run on the caller's STA.
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;

public static class OwnedInteractiveDesktop {
    [StructLayout(LayoutKind.Sequential)] struct Flags { public int inherit, reserved; public uint flags; }
    [DllImport("user32.dll")] static extern IntPtr GetProcessWindowStation();
    [DllImport("user32.dll")] static extern IntPtr GetThreadDesktop(uint id);
    [DllImport("kernel32.dll")] static extern uint GetCurrentThreadId();
    [DllImport("user32.dll")] static extern IntPtr OpenInputDesktop(uint flags, bool inherit, uint access);
    [DllImport("user32.dll")] static extern bool CloseDesktop(IntPtr desktop);
    [DllImport("user32.dll", EntryPoint="GetUserObjectInformationW")] static extern bool GetFlags(IntPtr obj, int index, ref Flags flags, uint length, out uint needed);
    [DllImport("user32.dll", EntryPoint="GetUserObjectInformationW", CharSet=CharSet.Unicode)] static extern bool GetName(IntPtr obj, int index, StringBuilder name, uint length, out uint needed);
    public static bool Current() {
        Flags flags = new Flags(); uint needed;
        if (!GetFlags(GetProcessWindowStation(), 1, ref flags, 12, out needed) || (flags.flags & 1) == 0) return false;
        IntPtr input = OpenInputDesktop(0, false, 1);
        if (input == IntPtr.Zero) return false;
        try {
            StringBuilder expected = new StringBuilder(256), current = new StringBuilder(256);
            return GetName(input, 2, expected, 512, out needed)
                && GetName(GetThreadDesktop(GetCurrentThreadId()), 2, current, 512, out needed)
                && expected.ToString() == current.ToString();
        } finally { CloseDesktop(input); }
    }
}

public sealed class OwnedClipboard : IDisposable {
    const uint UnicodeText = 13;
    const int MaximumFixtureBytes = 1048577;
    const ulong SnapshotLimit = 33554432;
    sealed class Entry { public uint Format; public byte[] Bytes; }
    sealed class Pending { public uint Format; public IntPtr Memory; }
    List<Entry> original;
    uint ownedSequence;
    // null means this controller successfully emptied the clipboard but its
    // subsequent SetClipboardData failed. That clear still has an exact revision.
    string ownedText;

    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern IntPtr CreateWindowExW(uint ex, string cls, string title, uint style, int x, int y, int width, int height, IntPtr parent, IntPtr menu, IntPtr instance, IntPtr param);
    [DllImport("user32.dll")] static extern bool DestroyWindow(IntPtr window);
    [DllImport("user32.dll")] static extern bool OpenClipboard(IntPtr window);
    [DllImport("user32.dll")] static extern bool CloseClipboard();
    [DllImport("user32.dll")] static extern bool EmptyClipboard();
    [DllImport("user32.dll")] static extern uint GetClipboardSequenceNumber();
    [DllImport("user32.dll")] static extern bool IsClipboardFormatAvailable(uint format);
    [DllImport("user32.dll")] static extern IntPtr GetClipboardData(uint format);
    [DllImport("user32.dll")] static extern IntPtr SetClipboardData(uint format, IntPtr memory);
    [DllImport("user32.dll", SetLastError=true)] static extern uint EnumClipboardFormats(uint format);
    [DllImport("kernel32.dll")] static extern void SetLastError(uint error);
    [DllImport("kernel32.dll")] static extern UIntPtr GlobalSize(IntPtr memory);
    [DllImport("kernel32.dll")] static extern IntPtr GlobalAlloc(uint flags, UIntPtr bytes);
    [DllImport("kernel32.dll")] static extern IntPtr GlobalFree(IntPtr memory);
    [DllImport("kernel32.dll")] static extern IntPtr GlobalLock(IntPtr memory);
    [DllImport("kernel32.dll")] static extern bool GlobalUnlock(IntPtr memory);

    sealed class Transaction : IDisposable {
        IntPtr window;
        bool opened;
        public Transaction() {
            if (!OwnedInteractiveDesktop.Current()) throw new InvalidOperationException("interactive_desktop_required");
            window = CreateWindowExW(0, "STATIC", "M117 private clipboard host", 0, 0, 0, 0, 0, IntPtr.Zero, IntPtr.Zero, IntPtr.Zero, IntPtr.Zero);
            if (window == IntPtr.Zero) throw new InvalidOperationException("clipboard_owner_window_failed");
            for (int attempt = 0; attempt < 10; attempt++) {
                if (OpenClipboard(window)) { opened = true; return; }
                Thread.Sleep(20);
            }
            DestroyWindow(window); window = IntPtr.Zero;
            throw new InvalidOperationException("clipboard_open_failed");
        }
        public void Dispose() {
            bool closed = !opened || CloseClipboard();
            if (window != IntPtr.Zero) DestroyWindow(window);
            window = IntPtr.Zero; opened = false;
            if (!closed) throw new InvalidOperationException("clipboard_close_failed");
        }
    }

    static byte[] ReadBytes(uint format, ulong maximum) {
        IntPtr memory = GetClipboardData(format);
        if (memory == IntPtr.Zero) throw new InvalidOperationException("clipboard_data_unavailable");
        ulong size = GlobalSize(memory).ToUInt64();
        if (size == 0 || size > maximum) throw new InvalidOperationException("clipboard_format_cannot_be_preserved");
        IntPtr pointer = GlobalLock(memory);
        if (pointer == IntPtr.Zero) throw new InvalidOperationException("clipboard_data_lock_failed");
        try {
            byte[] bytes = new byte[(int)size];
            Marshal.Copy(pointer, bytes, 0, bytes.Length);
            return bytes;
        } finally { GlobalUnlock(memory); }
    }
    static List<Entry> CaptureUnderLock() {
        List<Entry> entries = new List<Entry>();
        ulong total = 0;
        uint format = 0;
        while (true) {
            SetLastError(0);
            format = EnumClipboardFormats(format);
            if (format == 0) {
                if (Marshal.GetLastWin32Error() != 0) throw new InvalidOperationException("clipboard_enumeration_failed");
                return entries;
            }
            // Handle-based/GDI/private-owner formats cannot be recreated by a
            // copied HGLOBAL. Reject the whole snapshot before any modification.
            if (format == 2 || format == 3 || format == 9 || format == 14 || format == 0x80
                || format == 0x82 || format == 0x83 || format == 0x8e
                || (format >= 0x200 && format <= 0x3ff) || entries.Count >= 256) {
                throw new InvalidOperationException("prior_clipboard_format_unsupported");
            }
            byte[] bytes = ReadBytes(format, SnapshotLimit - total);
            total += (ulong)bytes.Length;
            entries.Add(new Entry { Format = format, Bytes = bytes });
        }
    }
    static IntPtr Allocate(byte[] bytes) {
        IntPtr memory = GlobalAlloc(2, new UIntPtr((uint)bytes.Length)); // GMEM_MOVEABLE
        if (memory == IntPtr.Zero) throw new InvalidOperationException("clipboard_allocate_failed");
        IntPtr pointer = GlobalLock(memory);
        if (pointer == IntPtr.Zero) { GlobalFree(memory); throw new InvalidOperationException("clipboard_allocate_lock_failed"); }
        bool copied = false;
        try { Marshal.Copy(bytes, 0, pointer, bytes.Length); copied = true; }
        finally { GlobalUnlock(memory); if (!copied) GlobalFree(memory); }
        return memory;
    }
    static uint Sequence() {
        uint sequence = GetClipboardSequenceNumber();
        if (sequence == 0) throw new InvalidOperationException("clipboard_sequence_unavailable");
        return sequence;
    }
    static string TextUnderLock() {
        if (!IsClipboardFormatAvailable(UnicodeText)) return null;
        byte[] bytes = ReadBytes(UnicodeText, (ulong)(MaximumFixtureBytes * 2 + 64));
        if ((bytes.Length & 1) != 0) throw new InvalidOperationException("clipboard_unicode_invalid");
        string text = Encoding.Unicode.GetString(bytes);
        int end = text.IndexOf('\0');
        if (end < 0) throw new InvalidOperationException("clipboard_unicode_unterminated");
        return text.Substring(0, end);
    }
    void WriteUnderLock(string text) {
        if (text.IndexOf('\0') >= 0 || Encoding.UTF8.GetByteCount(text) > MaximumFixtureBytes) {
            throw new InvalidOperationException("owned_marker_bound_failed");
        }
        Sequence(); // Refuse before modifying if revision ownership is unavailable.
        IntPtr memory = Allocate(Encoding.Unicode.GetBytes(text + "\0"));
        try {
            if (!EmptyClipboard()) throw new InvalidOperationException("clipboard_empty_failed");
            ownedSequence = Sequence(); ownedText = null;
            if (SetClipboardData(UnicodeText, memory) == IntPtr.Zero) throw new InvalidOperationException("clipboard_write_failed");
            memory = IntPtr.Zero; // Ownership transferred to Windows.
            ownedSequence = Sequence(); ownedText = text;
        } finally { if (memory != IntPtr.Zero) GlobalFree(memory); }
    }

    public bool HasSnapshot { get { return original != null; } }
    public void Begin(string initialMarker) {
        if (original != null) throw new InvalidOperationException("clipboard_session_already_begun");
        // Preserve and establish the first marker under the same lock. A user
        // copy cannot slip between snapshot capture and the first test change.
        using (Transaction transaction = new Transaction()) {
            original = CaptureUnderLock();
            WriteUnderLock(initialMarker);
        }
    }
    public void WriteText(string text) {
        if (original == null) throw new InvalidOperationException("clipboard_session_not_begun");
        using (Transaction transaction = new Transaction()) { WriteUnderLock(text); }
    }
    public bool EqualsText(string expected) {
        using (Transaction transaction = new Transaction()) {
            // An observed matching SDK write does not establish controller
            // ownership of its revision, including ambiguous empty text.
            return String.Equals(TextUnderLock(), expected, StringComparison.Ordinal);
        }
    }
    public bool RestoreIfOwned() {
        if (original == null || ownedSequence == 0) return false;
        using (Transaction transaction = new Transaction()) {
            // These checks and all restoration writes share ONE OpenClipboard
            // critical section. Same-value and empty user copies change the
            // sequence and are preserved; only direct controller writes qualify.
            if (Sequence() != ownedSequence || !String.Equals(TextUnderLock(), ownedText, StringComparison.Ordinal)) return false;
            List<Pending> staged = new List<Pending>();
            try {
                foreach (Entry entry in original) staged.Add(new Pending { Format = entry.Format, Memory = Allocate(entry.Bytes) });
                if (!EmptyClipboard()) throw new InvalidOperationException("clipboard_restore_empty_failed");
                foreach (Pending entry in staged) {
                    if (SetClipboardData(entry.Format, entry.Memory) == IntPtr.Zero) throw new InvalidOperationException("clipboard_restore_format_failed");
                    entry.Memory = IntPtr.Zero;
                }
                original = null; ownedSequence = 0; ownedText = null;
                return true;
            } finally {
                foreach (Pending entry in staged) if (entry.Memory != IntPtr.Zero) GlobalFree(entry.Memory);
            }
        }
    }
    public void Dispose() { original = null; ownedText = null; ownedSequence = 0; }
}
