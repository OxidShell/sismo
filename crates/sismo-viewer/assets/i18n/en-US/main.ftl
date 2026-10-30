## Common
common-open = Open
common-cancel = Cancel
common-close = Close
common-export = Export
common-state-needs-code = NEEDS CODE
common-space = Space

## Groups and tabs
group-app = App
group-framework = Framework
group-other = Other
tab-timeline = Timeline
tab-flamegraph = Flamegraph
tab-functions = Functions
tab-logs = Logs
tab-metrics = Metrics

## Toasts
toast-open-failed = Could not open
toast-connect-failed = Could not connect
toast-compare-failed = Could not compare
toast-recording-saved = Recording saved
toast-recording-saved-body = { $duration } · { $size } · { $markers } markers
toast-incompatible-format = Incompatible format
toast-incompatible-format-body = Update the viewer or use the same sismo version in the app.

## Sessions
sessions-title = Sessions
sessions-open-file = Open file…
sessions-seen-now = now
sessions-seen-ago = seen { $secs } s ago
sessions-state-incompatible = INCOMPATIBLE
sessions-state-connected = CONNECTED
sessions-state-available = AVAILABLE
sessions-action-details = Details
sessions-action-pair = Pair…
sessions-action-connect = Connect
sessions-incompatible-detail = { $platform } · uses sismo format v{ $theirs }, this viewer reads v{ $ours }
sessions-step-live-title = The app runs sismo in Live mode
sessions-step-live-body = SISMO=live or Config::live(..); it is off in release builds.
sessions-step-wifi-title = Same Wi-Fi as this PC
sessions-step-wifi-body = Guest networks and some routers block broadcast between devices.
sessions-step-firewall-title = The firewall allows UDP 7701
sessions-step-firewall-body = That is the discovery port; data uses 7720.
sessions-step-android-title = Android: INTERNET permission in the manifest
sessions-step-android-body = Without it the app cannot announce itself.
sessions-searching = Looking for apps…
sessions-none-found = No apps with sismo on this network
sessions-scan-failed = Scan failed: { $error }
sessions-no-network-hint = No network access to the client? Ask them to “Export diagnostics” and open the file on the right.
sessions-live-title = Live on the network
sessions-live-broadcast = broadcast zudp :7701 · { $count } apps
sessions-live-footer = Apps show up on their own once they enable sismo Live mode · Enter opens the selected one
sessions-open-sessions = Open sessions
sessions-files = Files
sessions-drop-title = Drop a .sismo here
sessions-drop-body = or press Ctrl O to pick one · also the ones a client exports
sessions-recent = Recent
sessions-tag-black-box = black box
sessions-tag-recorded = recorded
sessions-headless = Headless
sessions-cli-file = file

## Session window
shell-file-badge = file · { $duration }
shell-black-box-badge = BLACK BOX · panic · { $duration }
shell-damaged-badge = { $label } · truncated
shell-connecting = CONNECTING
shell-lost = DISCONNECTED
shell-paused = PAUSED
shell-live = LIVE
shell-rec-badge = REC { $clock } · { $size } · { $markers } markers
shell-marker = Marker  M
shell-resume = Resume
shell-pause = Pause
shell-stop-recording = Stop  Ctrl R
shell-record = Record…
shell-export = Export…
shell-time-share = % time
shell-threads = Threads
shell-status-live = { $rate } events/s   ·   { $dropped } dropped   ·   received { $received }   ·   zudp { $addr }   ·   rtt { $rtt }
shell-status-file = file · { $frames } frames · { $events } events · { $logs } logs · { $dropped } dropped · { $platform } · { $device } · { $version }
shell-app-filter = app filter: { $filter }
shell-waiting-connecting = Connecting…
shell-waiting-for = Waiting for { $addr }
shell-waiting-pairing = Pairing code required
shell-waiting-pairing-body = The app prints a 6-digit code in its log (target sismo).
shell-waiting-lost = Disconnected
shell-retry = Retry
shell-enter-code = Enter code…
shell-lost-banner = Connection lost ({ $reason }) · everything received up to { $until } is still here
shell-retry-now = Retry now
shell-save-received = Save what was received…
shell-paused-banner = View frozen · the app keeps sending and the viewer stores it in the background · Space resumes

## Query bar
query-group = group
query-thread-spans = { $count } spans
query-level-or-worse = that level or more severe
query-level-only = only that level
query-frame-slow = frames over 16.6 ms
query-placeholder = Filter · crate:app dur>2ms level:warn+ "text"
query-values = Values
query-filters = Filters
query-hint = ↑↓ choose · Tab complete · , adds another value · - in front excludes · ? syntax
common-show-in-functions = Show in Functions

## Timeline
timeline-no-frames = No frames: call sismo::frame_mark() or use the Freya plugin
timeline-span-location = { $file }:{ $line }   ·   crate { $krate } ({ $group })   ·   thread { $thread }
timeline-duration = Duration
timeline-self-time = Self time
timeline-children = Children
timeline-start = Start
timeline-allocs = Allocs
timeline-open-in-flamegraph = Open in Flamegraph
timeline-session-calls = In the session · { $count } calls · p50 { $p50 } · p95 { $p95 } · max { $max }
timeline-frames = Frames
timeline-frames-summary = { $count } frames   p50 { $p50 }   p95 { $p95 }   > 16.6 ms: { $slow }
timeline-hint-live = drag on frames = pick a range · right click = back to live · wheel = zoom · drag lanes = pan
timeline-hint-file = drag on frames = pick a range · right click = show all · wheel = zoom · drag lanes = pan
timeline-next-slow = Next slow  N
timeline-follow-live = Live  End
timeline-show-all = Show all

## Flamegraph
flame-empty = Nothing to show in this range with this query
flame-root = (root)
flame-total-own = total { $total } · self { $own }
flame-called-by = Called by
flame-calls = Calls
flame-own = (self)
flame-where-time-goes = Where does the time go?
flame-window-selection = selection
flame-window-session = whole session
flame-icicle = Icicle
flame-aggregate = aggregated · { $window } · { $threads } threads
flame-hint = width = share of time · click = zoom in · click the top bar or empty space = zoom out · right click = menu
flame-root = All
flame-up = Up one level  Esc

## Metrics
metrics-frame-time = Frame time
metrics-frame-summary = p95 { $p95 } · max { $max } · { $slow } slow
metrics-now = now { $now }
metrics-now-range = now { $now } · range { $lo }–{ $hi }
metrics-no-counters = The app recorded no counters · use sismo::counter!("name", value)

## Logs
common-show-in-timeline = Show in Timeline
common-count-of-total = { $count } of { $total }
logs-detail-location = { $time }  ·  { $target }  ·  { $file }:{ $line }  ·  thread { $thread }  ·  source { $source }
logs-fields = Fields
logs-no-fields = no fields
logs-inside = Inside
logs-outside-span = outside any span
logs-following = Following  End
logs-follow-paused = Following paused

## Functions
functions-col-function = Function
functions-col-calls = Calls
functions-col-total = Total
functions-col-self = Self
functions-col-p50 = p50
functions-col-p95 = p95
functions-col-max = Max
functions-col-alloc = Alloc
functions-col-history = p95 history
functions-detail-calls = { $count } calls · p50 { $p50 } · p95 { $p95 } · max { $max }
functions-detail-allocs = alloc { $own } self · { $total } total · { $per_call } per call · { $count } allocs
functions-history-caption = History · p95 per bucket
functions-worst-calls = Worst calls
functions-worst-hint = Click a call = open it in the Timeline
functions-filter-to = Filter to this function
functions-empty-title = Nothing matches “{ $query }”
functions-empty-body = Drop the strictest term or widen the range.
functions-clear-query = Clear query
functions-footer = { $count } of { $total } functions · click a header to sort · Enter opens the detail
functions-remove-compare = Remove comparison
functions-compare-new = new
functions-compare-gone = gone
functions-compare-footer = { $count } functions · red = worse in B · blue = better · gray = within noise (±10%) · sorted by absolute Δ

## Dialogs
toast-saved = Saved
toast-save-failed = Could not save
toast-cleared = Cleared, showing what arrives from now on
toast-clear-failed = Could not clear
toast-record-failed = Could not record
toast-exported = Exported
toast-export-failed = Could not export
toast-show-in-folder = Show in folder
dialog-pick-recording = Open sismo recording
dialog-pick-compare = Compare with…
dialog-save-received = Save what was received
dialog-file = File
dialog-file-path = file path
dialog-pair-title = Pair with { $app }
dialog-pair-body = Type the 6-digit code the app prints in its log (target sismo) or on its diagnostics screen.
dialog-pair-remembered = This viewer is remembered: next time it connects without a code.
dialog-pair-submit = Pair
dialog-record-title = Record session
dialog-record-body = Saves what arrives from the app on this PC, in the format the viewer and the CLI open.
dialog-record-from = From
dialog-record-include-buffer = Include what was already received
dialog-record-include-buffer-sub = { $size } since you connected, plus the app's buffer · useful to record something that already happened
dialog-record-from-now = Only from now on
dialog-record-stop = Stop
dialog-stop-manual = Manually
dialog-stop-5min = 5 min
dialog-stop-500mb = 500 MB
dialog-stop-first-error = On first error
dialog-record-first-error-hint = “On first error” keeps going 10 s longer to capture what came after.
dialog-record-on-device = Also record on the device
dialog-record-on-device-sub = The app writes its own copy and sends it when done; survives Wi-Fi drops.
dialog-record-start = Start recording  Ctrl R
dialog-format-perfetto = Perfetto  .pftrace
dialog-format-perfetto-sub = Full timeline in ui.perfetto.dev or SQL with trace_processor
dialog-format-summary = JSON summary  .json
dialog-format-summary-sub = For agents and scripts: top functions, slow frames, errors
dialog-format-markdown = Markdown report  .md
dialog-format-markdown-sub = Readable for an issue or a PR
dialog-format-chrome = Chrome trace  .json
dialog-format-chrome-sub = chrome://tracing and older tools
dialog-format-sismo = trimmed .sismo
dialog-format-sismo-sub = Only the chosen range, to send to someone else
dialog-export-format = Format
dialog-export-scope = Scope
dialog-export-scope-session = Whole session
dialog-export-scope-selection = Timeline selection
dialog-export-scope-query = What passes the current query
dialog-export-include-logs = Include logs
dialog-export-strip-fields = Strip log fields
dialog-export-strip-fields-sub = Useful if they may contain user data
dialog-filter-default = (default)
dialog-filter-title = What the app sends
dialog-filter-body = Applied on the device: lowering the level lowers the cost, not just what you see.
dialog-filter-placeholder = info,my_app=trace,zudp=warn
dialog-filter-levels = Levels per target
dialog-filter-apply = Apply in the app
dialog-clear-title = Clear this session?
dialog-clear-body = Everything received from { $app } so far ({ $size }, { $duration }) leaves this viewer. The app stays connected and new data shows up from now on.
dialog-clear-save-hint = "Save and clear" keeps a copy in { $folder }.
dialog-clear-discard = Clear
dialog-clear-save = Save and clear
shell-clear = Clear
dialog-marker-title = Marker
dialog-marker-placeholder = what are you doing
dialog-marker-body = Stays in the app and in the recording; shows up in Timeline, Logs and the CLI summary.
dialog-marker-save = Save  Enter

## Command palette
palette-placeholder = Search for an action or function…
palette-actions = Actions
palette-hint = Enter runs the first one · Esc closes
palette-compare = Compare with…
palette-record-toggle = Record / stop
palette-go-timeline = Go to Timeline
palette-go-flamegraph = Go to Flamegraph
palette-go-functions = Go to Functions
palette-go-logs = Go to Logs
palette-go-metrics = Go to Metrics
palette-app-filter = What the app sends…
palette-back-to-sessions = Back to Sessions
palette-shortcuts = Keyboard shortcuts

## Shortcuts
shortcuts-title = Shortcuts
shortcuts-general = General
shortcuts-session = Session
shortcuts-lists = Lists
shortcuts-palette = Command palette
shortcuts-open-file = Open file
shortcuts-close-session = Close session
shortcuts-these = These shortcuts
shortcuts-pause-resume = Pause / resume
shortcuts-zoom = Zoom in / out
shortcuts-pan = Pan
shortcuts-range = Range
shortcuts-key-drag = drag
shortcuts-next-slow = Next slow frame
shortcuts-back-to-live = Back to live
shortcuts-pick = Select
shortcuts-key-click = click
shortcuts-tab = Tab 1–5
shortcuts-clear-selection = Clear selection
