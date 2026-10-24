// Minimal typed content-only WebView2 host. No node or management bridge.
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Runtime.InteropServices;
using System.Security.Cryptography;
using System.Threading;
using System.Threading.Tasks;
using System.Web.Script.Serialization;
using System.Windows.Forms;
using Microsoft.Web.WebView2.Core;
using Microsoft.Web.WebView2.WinForms;

namespace Miopunch {
    public sealed class WebHostPin { public string path; public string sha256; }
    public sealed class WebHostOptions {
        public string gateway, gatewayCertificateSHA256, profile, runtimeDirectory, runtimeVersion;
        public WebHostPin coreAssembly, formsAssembly, loader, runtimeExecutable, hostAssembly;
    }
    public sealed class WebContentHost : Form {
        readonly WebHostOptions options;
        readonly WebView2 view = new WebView2();
        readonly System.Windows.Forms.Timer timer = new System.Windows.Forms.Timer();
        readonly JavaScriptSerializer json = new JavaScriptSerializer { MaxJsonLength = 131072 };
        readonly Stopwatch initialization = Stopwatch.StartNew();
        readonly TaskCompletionSource<bool> engineExited = new TaskCompletionSource<bool>();
        Task<string> input, evaluation;
        Stopwatch evaluationClock;
        object evaluationId;
        bool failed, ready, engineCreated, initializationFinished;
        readonly string origin;
        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        static extern IntPtr LoadLibraryEx(string file, IntPtr reserved, uint flags);

        WebContentHost(WebHostOptions options) {
            this.options = options;
            origin = new Uri(options.gateway).GetLeftPart(UriPartial.Authority);
            Width = 800; Height = 600; Text = "miopunch Web technical host";
            view.Dock = DockStyle.Fill; Controls.Add(view);
            timer.Interval = 20; timer.Tick += Tick; timer.Start();
            Shown += Initialize;
        }
        void Emit(object value) { Console.Out.WriteLine(json.Serialize(value)); Console.Out.Flush(); }
        bool SameOrigin(string value) {
            Uri parsed;
            return Uri.TryCreate(value, UriKind.Absolute, out parsed) && parsed.GetLeftPart(UriPartial.Authority) == origin;
        }
        void Fail(string phase, Exception error) {
            failed = true;
            Emit(new { @event = "host_failed", phase, error_class = error.GetType().Name });
            Close();
        }
        async void Initialize(object sender, EventArgs args) {
            try {
                // Normal dependency loading into THIS fresh host process;
                // never an external process, injection or a global DLL path.
                if (LoadLibraryEx(options.loader.path, IntPtr.Zero, 0x100 | 0x1000) == IntPtr.Zero)
                    throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
                var environmentOptions = new CoreWebView2EnvironmentOptions { AllowSingleSignOnUsingOSPrimaryAccount = false };
                var environment = await CoreWebView2Environment.CreateAsync(options.runtimeDirectory, options.profile, environmentOptions);
                environment.BrowserProcessExited += delegate { engineExited.TrySetResult(true); };
                engineCreated = true;
                await view.EnsureCoreWebView2Async(environment);
                if (environment.BrowserVersionString != options.runtimeVersion ||
                    !String.Equals(Path.GetFullPath(environment.UserDataFolder), Path.GetFullPath(options.profile), StringComparison.OrdinalIgnoreCase))
                    throw new InvalidOperationException("Unexpected private runtime or profile");
                var core = view.CoreWebView2;
                core.Settings.AreHostObjectsAllowed = false;
                core.Settings.IsWebMessageEnabled = false;
                core.Settings.AreDevToolsEnabled = false;
                core.Settings.AreDefaultContextMenusEnabled = false;
                core.Settings.AreDefaultScriptDialogsEnabled = false;
                core.Settings.IsPasswordAutosaveEnabled = false;
                core.Settings.IsGeneralAutofillEnabled = false;
                core.NavigationStarting += delegate(object source, CoreWebView2NavigationStartingEventArgs value) {
                    if (!SameOrigin(value.Uri)) { value.Cancel = true; Emit(new { @event = "navigation_denied" }); }
                };
                core.NewWindowRequested += delegate(object source, CoreWebView2NewWindowRequestedEventArgs value) { value.Handled = true; };
                core.PermissionRequested += delegate(object source, CoreWebView2PermissionRequestedEventArgs value) { value.State = CoreWebView2PermissionState.Deny; };
                core.DownloadStarting += delegate(object source, CoreWebView2DownloadStartingEventArgs value) { value.Cancel = true; };
                core.ServerCertificateErrorDetected += CertificateError;
                core.Navigate(options.gateway);
                input = Task.Run(() => Console.In.ReadLine());
                ready = true;
                Emit(new { @event = "ready", engine = "WebView2", native_management_handlers = false, profile_fresh = true });
            } catch (Exception error) { Fail("initialization", error); }
            finally { initializationFinished = true; }
        }
        void CertificateError(object sender, CoreWebView2ServerCertificateErrorDetectedEventArgs value) {
            string digest;
            using (var certificate = value.ServerCertificate.ToX509Certificate2())
            using (var hash = SHA256.Create())
                digest = BitConverter.ToString(hash.ComputeHash(certificate.RawData)).Replace("-", "").ToLowerInvariant();
            value.Action = SameOrigin(value.RequestUri) && digest == options.gatewayCertificateSHA256
                ? CoreWebView2ServerCertificateErrorAction.AlwaysAllow : CoreWebView2ServerCertificateErrorAction.Cancel;
        }
        void Tick(object sender, EventArgs args) {
            try {
                if (!ready) { if (initialization.Elapsed.TotalSeconds >= 10) throw new TimeoutException("Content initialization"); return; }
                if (evaluation != null) {
                    if (evaluation.IsCompleted) {
                        Emit(new { id = evaluationId, ok = true, result = json.DeserializeObject(evaluation.GetAwaiter().GetResult()) });
                        evaluation = null;
                    } else if (evaluationClock.Elapsed.TotalSeconds >= 5) throw new TimeoutException("Content evaluation");
                }
                if (evaluation == null && input != null && input.IsCompleted) {
                    string line = input.GetAwaiter().GetResult();
                    if (line == null) { Close(); return; }
                    if (line.Length > 65536) throw new InvalidOperationException("Private control bound");
                    var command = json.Deserialize<Dictionary<string, object>>(line);
                    string operation = (string)command["op"];
                    if (operation == "close") { Emit(new { id = command["id"], ok = true, result = new { closed = true } }); Close(); return; }
                    if (operation != "evaluate") throw new InvalidOperationException("Unsupported private operation");
                    evaluationId = command["id"]; evaluationClock = Stopwatch.StartNew();
                    evaluation = view.CoreWebView2.ExecuteScriptAsync("(" + (string)command["expression"] + ")");
                    input = Task.Run(() => Console.In.ReadLine());
                }
            } catch (Exception error) { Fail("private_control", error); }
        }
        void Finish() {
            timer.Stop(); timer.Dispose();
            view.Dispose();
            // The owner closes its original stdin on this event. No OS signal
            // or unrelated process is touched by this content-only host.
            Emit(new { @event = "closing", caller_must_close_owned_stdin = true, caller_must_join_gateway = true });
            var deadline = Stopwatch.StartNew();
            while (deadline.Elapsed.TotalSeconds < 5 &&
                   ((input != null && !input.IsCompleted) || !initializationFinished || (engineCreated && !engineExited.Task.IsCompleted))) {
                Application.DoEvents(); Thread.Sleep(10);
            }
            bool inputJoined = input == null || input.IsCompleted;
            bool engineJoined = initializationFinished && (!engineCreated || engineExited.Task.IsCompleted);
            failed |= !inputJoined || !engineJoined;
            Emit(new { @event = "closed", host_failed = failed, input_joined = inputJoined, engine_exit_observed = engineJoined });
        }
        public static int Run(string optionsJson) {
            var options = new JavaScriptSerializer().Deserialize<WebHostOptions>(optionsJson);
            using (var host = new WebContentHost(options)) {
                try { Application.Run(host); }
                finally { host.Finish(); }
                return host.failed ? 1 : 0;
            }
        }
    }
}
