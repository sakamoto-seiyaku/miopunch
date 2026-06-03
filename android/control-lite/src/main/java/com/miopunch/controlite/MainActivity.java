package com.miopunch.controlite;

import android.app.Activity;
import android.content.Intent;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.text.InputType;
import android.util.Log;
import android.view.View;
import android.webkit.JavascriptInterface;
import android.webkit.WebResourceRequest;
import android.webkit.WebView;
import android.webkit.WebViewClient;
import android.widget.ArrayAdapter;
import android.widget.Button;
import android.widget.EditText;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.Spinner;
import android.widget.TextView;

import java.io.BufferedWriter;
import java.io.File;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStreamWriter;
import java.nio.charset.StandardCharsets;
import java.text.SimpleDateFormat;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Date;
import java.util.List;
import java.util.Locale;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

public final class MainActivity extends Activity {
    private static final String TAG = "MiopunchControlLite";
    private static final int MAX_LOG_LINES = 500;
    private static final int MAX_PENDING_TERMINAL_WRITES = 200;

    private final Handler main = new Handler(Looper.getMainLooper());
    private final ExecutorService io = Executors.newCachedThreadPool();
    private final List<String> logLines = new ArrayList<>();
    private final List<String> pendingTerminalWrites = new ArrayList<>();

    private EditText inviteInput;
    private EditText peerInput;
    private EditText targetInput;
    private EditText sessionInput;
    private EditText shellInput;
    private Spinner p2pNetworkInput;
    private Spinner p2pIPFamilyInput;
    private TextView logView;
    private ScrollView logScroll;
    private WebView shellTerminal;
    private Button startButton;
    private Button stopButton;
    private Button joinButton;
    private Button lsButton;
    private Button pingButton;
    private Button shellLsButton;
    private Button openShellButton;
    private Button sendShellButton;

    private File miopunch;
    private File localAPISocket;
    private File stateFile;
    private File logsDir;

    private Process runtimeProcess;
    private Process shellProcess;
    private BufferedWriter shellWriter;
    private boolean terminalReady;

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);

        miopunch = new File(getApplicationInfo().nativeLibraryDir, "libmiopunch.so");
        localAPISocket = new File(getCacheDir(), "miopunch-localapi.sock");
        stateFile = new File(new File(getFilesDir(), "state"), "state.json");
        logsDir = new File(getFilesDir(), "logs");

        setContentView(buildUI());
        applyIntentExtras(getIntent());
        updateControls();
        appendLog("miopunch=" + miopunch.getAbsolutePath());
        appendLog("localapi=" + localAPIAddr());
        runHelpCheck();
    }

    @Override
    protected void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
        setIntent(intent);
        applyIntentExtras(intent);
    }

    @Override
    protected void onDestroy() {
        stopAll();
        io.shutdownNow();
        super.onDestroy();
    }

    private View buildUI() {
        LinearLayout root = new LinearLayout(this);
        root.setOrientation(LinearLayout.VERTICAL);
        int pad = dp(12);
        root.setPadding(pad, pad, pad, pad);
        root.setLayoutParams(new LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT,
                LinearLayout.LayoutParams.MATCH_PARENT
        ));

        inviteInput = input("Invite code", false);
        peerInput = input("Peer ID", false);
        targetInput = input("Target", false);
        targetInput.setText("local");
        sessionInput = input("Session", false);
        sessionInput.setText("main");
        root.addView(inviteInput, fillWrap());
        root.addView(peerInput, fillWrap());
        root.addView(targetInput, fillWrap());
        root.addView(sessionInput, fillWrap());

        root.addView(label("P2P path"), fillWrap());
        LinearLayout pathRow = row();
        p2pNetworkInput = spinner("auto", "udp_only", "tcp_only");
        p2pIPFamilyInput = spinner("auto", "v4", "v6");
        pathRow.addView(p2pNetworkInput, weight());
        pathRow.addView(p2pIPFamilyInput, weight());
        root.addView(pathRow, fillWrap());

        LinearLayout row1 = row();
        startButton = button("Start Runtime", v -> startRuntime());
        stopButton = button("Stop", v -> stopAll());
        row1.addView(startButton, weight());
        row1.addView(stopButton, weight());
        root.addView(row1, fillWrap());

        LinearLayout row2 = row();
        joinButton = button("Join", v -> joinNetwork());
        lsButton = button("LS", v -> runLS());
        pingButton = button("Ping", v -> runPing());
        row2.addView(joinButton, weight());
        row2.addView(lsButton, weight());
        row2.addView(pingButton, weight());
        root.addView(row2, fillWrap());

        LinearLayout row3 = row();
        shellLsButton = button("Shell LS", v -> runShellLS());
        openShellButton = button("Open Shell", v -> openShell());
        row3.addView(shellLsButton, weight());
        row3.addView(openShellButton, weight());
        root.addView(row3, fillWrap());

        root.addView(label("Logs"), fillWrap());
        logView = output();
        logScroll = outputPanel(logView);
        root.addView(logScroll, weightedPanel(2));

        root.addView(label("Shell"), fillWrap());
        shellTerminal = terminalView();
        root.addView(shellTerminal, weightedPanel(1));

        LinearLayout shellRow = row();
        shellInput = input("Shell command", false);
        shellInput.setSingleLine(true);
        sendShellButton = button("Send", v -> sendShellLine());
        shellRow.addView(shellInput, new LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 3));
        shellRow.addView(sendShellButton, weight());
        root.addView(shellRow, fillWrap());

        return root;
    }

    private EditText input(String hint, boolean multiLine) {
        EditText e = new EditText(this);
        e.setHint(hint);
        e.setTextSize(14);
        if (multiLine) {
            e.setInputType(InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_FLAG_MULTI_LINE);
            e.setSingleLine(false);
        } else {
            e.setSingleLine(true);
        }
        return e;
    }

    private TextView label(String title) {
        TextView v = new TextView(this);
        v.setText(title);
        v.setTextSize(13);
        v.setPadding(0, dp(4), 0, dp(2));
        return v;
    }

    private TextView output() {
        TextView v = new TextView(this);
        v.setText("");
        v.setTextSize(13);
        v.setTextIsSelectable(true);
        v.setPadding(dp(8), dp(8), dp(8), dp(8));
        return v;
    }

    private ScrollView outputPanel(TextView output) {
        ScrollView scroll = new ScrollView(this);
        scroll.setFillViewport(false);
        scroll.addView(output, fillWrap());
        return scroll;
    }

    private WebView terminalView() {
        WebView terminal = new WebView(this);
        terminal.setBackgroundColor(0xff0b0b0f);
        terminal.getSettings().setJavaScriptEnabled(true);
        terminal.getSettings().setAllowContentAccess(false);
        terminal.getSettings().setAllowFileAccess(true);
        terminal.getSettings().setAllowFileAccessFromFileURLs(false);
        terminal.getSettings().setAllowUniversalAccessFromFileURLs(false);
        terminal.setWebViewClient(new WebViewClient() {
            @Override
            public boolean shouldOverrideUrlLoading(WebView view, WebResourceRequest request) {
                return true;
            }
        });
        terminal.addJavascriptInterface(new TerminalBridge(), "MiopunchTerminal");
        terminal.loadUrl("file:///android_asset/terminal/index.html");
        return terminal;
    }

    private Button button(String label, View.OnClickListener listener) {
        Button b = new Button(this);
        b.setText(label);
        b.setAllCaps(false);
        b.setOnClickListener(listener);
        return b;
    }

    private Spinner spinner(String... values) {
        Spinner s = new Spinner(this);
        ArrayAdapter<String> adapter = new ArrayAdapter<>(this, android.R.layout.simple_spinner_item, values);
        adapter.setDropDownViewResource(android.R.layout.simple_spinner_dropdown_item);
        s.setAdapter(adapter);
        return s;
    }

    private LinearLayout row() {
        LinearLayout row = new LinearLayout(this);
        row.setOrientation(LinearLayout.HORIZONTAL);
        return row;
    }

    private LinearLayout.LayoutParams fillWrap() {
        return new LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT,
                LinearLayout.LayoutParams.WRAP_CONTENT
        );
    }

    private LinearLayout.LayoutParams weight() {
        return new LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1);
    }

    private LinearLayout.LayoutParams weightedPanel(int weight) {
        return new LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, 0, weight);
    }

    private int dp(int value) {
        return Math.round(value * getResources().getDisplayMetrics().density);
    }

    private void applyIntentExtras(Intent intent) {
        if (intent == null) {
            return;
        }
        setTextExtra(inviteInput, intent, "invite");
        setTextExtra(peerInput, intent, "peer");
        setTextExtra(targetInput, intent, "target");
        setTextExtra(sessionInput, intent, "session");
        setSpinnerExtra(p2pNetworkInput, intent, "p2p_network");
        setSpinnerExtra(p2pIPFamilyInput, intent, "p2p_ip_family");
        if (intent.getBooleanExtra("open_shell", false)) {
            appendLog("intent open_shell=true");
            main.postDelayed(() -> openShell(), 100);
        }
        String shellLine = intent.getStringExtra("shell_line");
        if (shellLine == null) {
            shellLine = intent.getStringExtra("line");
        }
        if (shellLine != null) {
            appendLog("intent shell_line: " + shellLine);
            shellInput.setText(shellLine);
            main.postDelayed(() -> sendShellLine(), 100);
        }
    }

    private void setSpinnerExtra(Spinner input, Intent intent, String key) {
        String value = intent.getStringExtra(key);
        if (value == null) {
            return;
        }
        selectSpinner(input, value);
    }

    private void selectSpinner(Spinner input, String value) {
        if (input == null) {
            return;
        }
        String wanted = value == null ? "" : value.trim();
        for (int i = 0; i < input.getCount(); i++) {
            Object item = input.getItemAtPosition(i);
            if (item != null && wanted.equals(item.toString())) {
                input.setSelection(i);
                return;
            }
        }
    }

    private void setTextExtra(EditText input, Intent intent, String key) {
        String value = intent.getStringExtra(key);
        if (value != null) {
            input.setText(value);
        }
    }

    private void runHelpCheck() {
        io.execute(() -> {
            int rc = runProcess("payload", null, logFile("payload.log"), Arrays.asList(miopunchPath(), "--help"));
            appendLog("payload --help exit=" + rc);
        });
    }

    private void startRuntime() {
        if (runtimeRunning()) {
            appendLog("runtime already running");
            return;
        }
        if (!ensureDirs()) {
            return;
        }
        if (localAPISocket.exists() && !localAPISocket.delete()) {
            appendLog("warn: failed to remove stale localapi socket");
        }

        List<String> cmd = Arrays.asList(
                miopunchPath(),
                "up",
                "--localapi", localAPIAddr(),
                "--state_path", stateFile.getAbsolutePath(),
                "--log-level", "trace"
        );

        try {
            ProcessBuilder pb = new ProcessBuilder(cmd);
            pb.directory(getFilesDir());
            Process process = pb.start();
            runtimeProcess = process;
            appendLog("$ " + joinCommand(cmd));
            stream(process.getInputStream(), logView, logFile("runtime.stdout.log"), "[runtime] ");
            stream(process.getErrorStream(), logView, logFile("runtime.stderr.log"), "[runtime err] ");
            io.execute(() -> {
                int rc = waitFor(process);
                appendLog("runtime exited rc=" + rc);
                removeLocalAPISocket("runtime exit");
                if (runtimeProcess == process) {
                    runtimeProcess = null;
                }
                updateControls();
            });
        } catch (IOException e) {
            appendLog("start runtime failed: " + e.getMessage());
            runtimeProcess = null;
        }
        updateControls();
    }

    private void stopAll() {
        closeShellWriter();
        Process activeShell = shellProcess;
        shellProcess = null;
        destroy(activeShell);
        Process activeRuntime = runtimeProcess;
        runtimeProcess = null;
        destroy(activeRuntime);
        if (activeRuntime == null) {
            removeLocalAPISocket("stop");
        }
        appendLog("stopped");
        updateControls();
    }

    private void joinNetwork() {
        String code = text(inviteInput);
        if (code.isEmpty()) {
            appendLog("missing invite code");
            return;
        }
        runAction("join", "join", code);
    }

    private void runLS() {
        runAction("ls", "ls");
    }

    private void runPing() {
        String peer = text(peerInput);
        if (peer.isEmpty()) {
            appendLog("missing peer id");
            return;
        }
        List<String> args = new ArrayList<>(Arrays.asList("ping", peer));
        args.addAll(p2pArgs());
        runAction("ping", args);
    }

    private void runShellLS() {
        String peer = text(peerInput);
        if (peer.isEmpty()) {
            appendLog("missing peer id");
            return;
        }
        List<String> args = new ArrayList<>(Arrays.asList("sh", "ls", peer, target()));
        args.addAll(p2pArgs());
        runAction("sh ls", args);
    }

    private void runAction(String label, String... args) {
        runAction(label, Arrays.asList(args));
    }

    private void runAction(String label, List<String> args) {
        if (!runtimeRunning()) {
            appendLog("runtime is stopped");
            updateControls();
            return;
        }
        List<String> cmd = cliCommand(args);
        io.execute(() -> {
            int rc = runProcess(label, logView, logFile(safeName(label) + ".log"), cmd);
            appendLog(label + " exit=" + rc);
        });
    }

    private void openShell() {
        if (!runtimeRunning()) {
            appendLog("runtime is stopped");
            return;
        }
        if (shellRunning()) {
            appendLog("shell already running");
            return;
        }
        String peer = text(peerInput);
        if (peer.isEmpty()) {
            appendLog("missing peer id");
            return;
        }
        if (!ensureDirs()) {
            return;
        }

        List<String> cmd = new ArrayList<>();
        cmd.add(miopunchPath());
        cmd.add("--localapi");
        cmd.add(localAPIAddr());
        cmd.add("sh");
        cmd.add(peer);
        cmd.add(target());
        cmd.add("-s");
        cmd.add(sessionName());
        cmd.addAll(p2pArgs());

        try {
            ProcessBuilder pb = new ProcessBuilder(cmd);
            pb.directory(getFilesDir());
            Process process = pb.start();
            shellProcess = process;
            shellWriter = new BufferedWriter(new OutputStreamWriter(process.getOutputStream(), StandardCharsets.UTF_8));
            appendLog("$ " + joinCommand(cmd));
            clearTerminal();
            appendShell("shell opened: " + peer + " " + target() + "\n");
            streamShell(process.getInputStream(), logFile("shell.stdout.log"));
            stream(process.getErrorStream(), logView, logFile("shell.stderr.log"), "[shell err] ");
            io.execute(() -> {
                int rc = waitFor(process);
                appendLog("shell exited rc=" + rc);
                closeShellWriter();
                if (shellProcess == process) {
                    shellProcess = null;
                }
                updateControls();
            });
        } catch (IOException e) {
            appendLog("open shell failed: " + e.getMessage());
            closeShellWriter();
            shellProcess = null;
        }
        updateControls();
    }

    private void sendShellLine() {
        if (!shellRunning() || shellWriter == null) {
            appendLog("shell is not running");
            updateControls();
            return;
        }
        String line = shellInput.getText().toString();
        shellInput.setText("");
        writeShellInput(line + "\r", true);
    }

    private void writeShellInput(String data, boolean logLine) {
        if (data.isEmpty()) {
            return;
        }
        if (!shellRunning() || shellWriter == null) {
            appendLog("shell is not running");
            updateControls();
            return;
        }
        io.execute(() -> {
            try {
                shellWriter.write(data);
                shellWriter.flush();
                if (logLine) {
                    appendLog("shell sent: " + shortLine(data, 96));
                }
            } catch (IOException e) {
                appendLog("shell write failed: " + e.getMessage());
            }
        });
    }

    private int runProcess(String label, TextView target, File logFile, List<String> cmd) {
        if (!ensureDirs()) {
            return 1;
        }
        appendLog("$ " + joinCommand(cmd));
        try {
            ProcessBuilder pb = new ProcessBuilder(cmd);
            pb.directory(getFilesDir());
            Process p = pb.start();
            stream(p.getInputStream(), target, logFile, "[" + label + "] ");
            stream(p.getErrorStream(), target == null ? null : logView, logFile, "[" + label + " err] ");
            return waitFor(p);
        } catch (IOException e) {
            appendLog(label + " failed: " + e.getMessage());
            return 1;
        }
    }

    private void stream(InputStream in, TextView view, File logFile, String prefix) {
        io.execute(() -> {
            byte[] buf = new byte[4096];
            try (FileOutputStream out = new FileOutputStream(logFile, true)) {
                int n;
                while ((n = in.read(buf)) != -1) {
                    String text = new String(buf, 0, n, StandardCharsets.UTF_8);
                    String rendered = prefix.isEmpty() ? text : prefix + text;
                    out.write(rendered.getBytes(StandardCharsets.UTF_8));
                    append(view, rendered);
                }
            } catch (IOException e) {
                if ("read interrupted by close() on another thread".equals(e.getMessage())) {
                    return;
                }
                appendLog("stream failed: " + e.getMessage());
            }
        });
    }

    private void streamShell(InputStream in, File logFile) {
        io.execute(() -> {
            byte[] buf = new byte[4096];
            try (FileOutputStream out = new FileOutputStream(logFile, true)) {
                int n;
                while ((n = in.read(buf)) != -1) {
                    String text = new String(buf, 0, n, StandardCharsets.UTF_8);
                    out.write(text.getBytes(StandardCharsets.UTF_8));
                    appendShell(text);
                }
            } catch (IOException e) {
                if ("read interrupted by close() on another thread".equals(e.getMessage())) {
                    return;
                }
                appendLog("shell stream failed: " + e.getMessage());
            }
        });
    }

    private int waitFor(Process process) {
        if (process == null) {
            return 0;
        }
        try {
            return process.waitFor();
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            return 130;
        }
    }

    private void destroy(Process process) {
        if (process == null) {
            return;
        }
        process.destroy();
    }

    private void closeShellWriter() {
        if (shellWriter == null) {
            return;
        }
        try {
            shellWriter.close();
        } catch (IOException ignored) {
        }
        shellWriter = null;
    }

    private void removeLocalAPISocket(String context) {
        if (localAPISocket.exists() && !localAPISocket.delete()) {
            appendLog(context + ": localapi socket cleanup deferred");
        }
    }

    private boolean ensureDirs() {
        if (!logsDir.exists() && !logsDir.mkdirs()) {
            appendLog("failed to create logs dir: " + logsDir.getAbsolutePath());
            return false;
        }
        File stateDir = stateFile.getParentFile();
        if (stateDir != null && !stateDir.exists() && !stateDir.mkdirs()) {
            appendLog("failed to create state dir: " + stateDir.getAbsolutePath());
            return false;
        }
        return true;
    }

    private File logFile(String name) {
        ensureDirs();
        return new File(logsDir, safeName(name));
    }

    private String safeName(String value) {
        String v = value.replaceAll("[^A-Za-z0-9._-]+", "-");
        if (v.isEmpty()) {
            return "miopunch.log";
        }
        return v;
    }

    private List<String> cliCommand(String... args) {
        return cliCommand(Arrays.asList(args));
    }

    private List<String> cliCommand(List<String> args) {
        List<String> cmd = new ArrayList<>();
        cmd.add(miopunchPath());
        cmd.add("--localapi");
        cmd.add(localAPIAddr());
        cmd.add("--format");
        cmd.add("json");
        cmd.add("--redact");
        cmd.addAll(args);
        return cmd;
    }

    private List<String> p2pArgs() {
        List<String> args = new ArrayList<>();
        String network = selected(p2pNetworkInput);
        if ("udp_only".equals(network)) {
            args.add("-u");
        } else if ("tcp_only".equals(network)) {
            args.add("-t");
        }
        String family = selected(p2pIPFamilyInput);
        if ("v4".equals(family)) {
            args.add("-4");
        } else if ("v6".equals(family)) {
            args.add("-6");
        }
        return args;
    }

    private String selected(Spinner input) {
        if (input == null) {
            return "";
        }
        Object value = input.getSelectedItem();
        return value == null ? "" : value.toString().trim();
    }

    private String miopunchPath() {
        return miopunch.getAbsolutePath();
    }

    private String localAPIAddr() {
        return "unix:" + localAPISocket.getAbsolutePath();
    }

    private String target() {
        String value = text(targetInput);
        return value.isEmpty() ? "local" : value;
    }

    private String sessionName() {
        String value = text(sessionInput);
        return value.isEmpty() ? "main" : value;
    }

    private String text(EditText e) {
        return e.getText().toString().trim();
    }

    private boolean runtimeRunning() {
        return runtimeProcess != null && runtimeProcess.isAlive();
    }

    private boolean shellRunning() {
        return shellProcess != null && shellProcess.isAlive();
    }

    private void updateControls() {
        main.post(() -> renderControls());
    }

    private void renderControls() {
        boolean runtime = runtimeRunning();
        boolean shell = shellRunning();
        startButton.setEnabled(!runtime);
        stopButton.setEnabled(runtime || shell);
        joinButton.setEnabled(runtime);
        lsButton.setEnabled(runtime);
        pingButton.setEnabled(runtime);
        shellLsButton.setEnabled(runtime);
        openShellButton.setEnabled(runtime && !shell);
        sendShellButton.setEnabled(shell);
    }

    private void appendLog(String message) {
        Log.i(TAG, message);
        String line = "[" + stamp() + "] " + message + "\n";
        main.post(() -> appendBuffered(logView, logScroll, logLines, line, MAX_LOG_LINES));
    }

    private void appendShell(String message) {
        writeTerminal(message);
    }

    private void append(TextView view, String message) {
        if (view == null) {
            return;
        }
        main.post(() -> appendBuffered(logView, logScroll, logLines, message, MAX_LOG_LINES));
    }

    private void clearTerminal() {
        main.post(() -> {
            pendingTerminalWrites.clear();
            if (terminalReady && shellTerminal != null) {
                shellTerminal.evaluateJavascript("window.miopunchClear && window.miopunchClear();", null);
            }
        });
    }

    private void writeTerminal(String message) {
        main.post(() -> {
            if (!terminalReady || shellTerminal == null) {
                pendingTerminalWrites.add(message);
                while (pendingTerminalWrites.size() > MAX_PENDING_TERMINAL_WRITES) {
                    pendingTerminalWrites.remove(0);
                }
                return;
            }
            shellTerminal.evaluateJavascript(
                    "window.miopunchWrite && window.miopunchWrite(" + jsString(message) + ");",
                    null
            );
        });
    }

    private void flushTerminalWrites() {
        if (!terminalReady || shellTerminal == null || pendingTerminalWrites.isEmpty()) {
            return;
        }
        StringBuilder b = new StringBuilder();
        for (String text : pendingTerminalWrites) {
            b.append(text);
        }
        pendingTerminalWrites.clear();
        shellTerminal.evaluateJavascript(
                "window.miopunchWrite && window.miopunchWrite(" + jsString(b.toString()) + ");",
                null
        );
    }

    private String jsString(String value) {
        StringBuilder b = new StringBuilder(value.length() + 16);
        b.append('"');
        for (int i = 0; i < value.length(); i++) {
            char c = value.charAt(i);
            switch (c) {
                case '\\':
                    b.append("\\\\");
                    break;
                case '"':
                    b.append("\\\"");
                    break;
                case '\b':
                    b.append("\\b");
                    break;
                case '\f':
                    b.append("\\f");
                    break;
                case '\n':
                    b.append("\\n");
                    break;
                case '\r':
                    b.append("\\r");
                    break;
                case '\t':
                    b.append("\\t");
                    break;
                default:
                    if (c < 0x20 || c == 0x2028 || c == 0x2029) {
                        b.append("\\u");
                        String hex = Integer.toHexString(c);
                        for (int j = hex.length(); j < 4; j++) {
                            b.append('0');
                        }
                        b.append(hex);
                    } else {
                        b.append(c);
                    }
                    break;
            }
        }
        b.append('"');
        return b.toString();
    }

    private final class TerminalBridge {
        @JavascriptInterface
        public void ready() {
            main.post(() -> {
                terminalReady = true;
                flushTerminalWrites();
            });
        }

        @JavascriptInterface
        public void send(String data) {
            writeShellInput(data == null ? "" : data, false);
        }
    }

    private void appendBuffered(TextView view, ScrollView scroll, List<String> lines, String message, int maxLines) {
        appendLines(lines, message);
        while (lines.size() > maxLines) {
            lines.remove(0);
        }
        StringBuilder b = new StringBuilder();
        for (String line : lines) {
            b.append(line);
        }
        view.setText(b.toString());
        scroll.post(() -> scroll.fullScroll(View.FOCUS_DOWN));
    }

    private void appendLines(List<String> lines, String message) {
        int start = 0;
        for (int i = 0; i < message.length(); i++) {
            if (message.charAt(i) == '\n') {
                lines.add(message.substring(start, i + 1));
                start = i + 1;
            }
        }
        if (start < message.length()) {
            lines.add(message.substring(start));
        }
    }

    private String shortLine(String message, int maxLen) {
        String v = message.replace('\n', ' ').replace('\r', ' ').trim();
        if (v.length() <= maxLen) {
            return v;
        }
        return v.substring(0, Math.max(0, maxLen - 3)) + "...";
    }

    private String stamp() {
        return new SimpleDateFormat("HH:mm:ss", Locale.US).format(new Date());
    }

    private String joinCommand(List<String> cmd) {
        StringBuilder b = new StringBuilder();
        for (String part : cmd) {
            if (b.length() > 0) {
                b.append(' ');
            }
            String rendered = displayArg(part);
            b.append(rendered.indexOf(' ') >= 0 ? "'" + rendered + "'" : rendered);
        }
        return b.toString();
    }

    private String displayArg(String value) {
        if (value.startsWith("MPINV1-")) {
            return "MPINV1-<redacted>";
        }
        if (value.length() > 96) {
            return value.substring(0, 93) + "...";
        }
        return value;
    }
}
