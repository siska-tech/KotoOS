# KotoOS Resource Ownership for PicoCalc

## 0. Scope

この文書は、KotoOS/PicoCalc/RP2040向けのリソース所有権モデルを定義する。CPU、DMA、PIO、SPI、PSRAM、SD、Audio、Display、Input、Transportなどの物理資源を誰が所有し、通常アプリ、システムアプリ、HAL、デバッグビルドへどこまで公開するかを明文化する。

本設計は `docs/research/piece-analysis/02b_resource_reservation_model.md` の調査結果をもとにする。ただし、P/ECE互換APIは作らない。P/ECEのコード、ヘッダ、文書はコピーせず、KotoOS独自の設計文書としてまとめる。

前提:

- KotoOSはPicoCalc/RP2040向けに設計する。
- RP2040はdual-coreである。
- LCDはSPI接続である。
- PSRAMは8MBを想定する。
- SDカードは標準搭載とする。
- 音声はCPU制御を前提にする。
- KotoVM、koto-gfx、koto-audio、koto-input、koto-storage、koto-psramを分離設計中である。
- 通常アプリはraw hardwareへ直接アクセスしない。
- 物理資源はHALまたはsystem serviceが所有する。
- アプリはhostcall/APIで論理操作だけを要求する。

## 1. Resource Ownership Principle

KotoOSでは、物理資源はkernel、HAL、またはsystem serviceの所有物として扱う。通常アプリへraw hardwareを貸さない。SPI、DMA channel、PIO state machine、hardware timer、GPIO scan path、SD bus、UART/USB endpointなどは通常アプリの所有対象ではない。

基本原則は、physical resourceとlogical objectを分離することである。

- Physical resource: CPU core、DMA channel、PIO state machine、SPI bus、PSRAM bus、SD bus、hardware timer、UART/USB transport、PWM output、GPIO scan path。
- Logical object: app surface、audio source、input event stream、save file、app bundle、timer event、debug log channel、virtual transport channel。

通常アプリはlogical objectを作成・操作できるが、その実現にどのphysical resourceを使うかはsystem serviceとHALが決める。たとえばdisplay present requestはSPIやDMAの予約ではなく、display serviceへの要求である。audio sourceはPWM、PIO、DMAを所有せず、audio mixerへ投入される。save writeはSD busを所有せず、koto-storageの管理するtransactionに入る。

system serviceは優先度と競合を管理する。競合時にアプリ同士が物理資源を奪い合うのではなく、system serviceがdeadline、priority、focus、foreground/background状態、power状態を見て処理順を決める。

debug buildだけは低レイヤ観測口を持つ。DMA/PIO/SPI/timerの予約状況、queue depth、latency、underrun、raw scan、transport traceなどを観測できるが、これは診断用であり通常アプリABIではない。

## 2. Resource Ownership Table

| Resource | Owner | Priority | Normal app access | System app access | Debug access | Public hostcall/API | Conflict policy |
|---|---|---:|---|---|---|---|---|
| CPU0 | KotoOS kernel + KotoVM app scheduler | Critical | scheduler経由でVM sliceを実行するだけ | foreground/background policyを要求可能 | scheduler trace、tick latency、app budget | `yield`、`sleep`、timer event、app lifecycle | CPU0はVM/app scheduler中心に固定する。長時間占有はpreempt、throttle、stopの対象にする。 |
| CPU1 | buildまたはboot policyで選ばれた単一system owner | Critical | 直接アクセス不可 | 単一service groupだけが所有 | core load、queue depth、deadline miss | 通常アプリ向け公開なし | 初期版は所有者固定。display/audio/storage/transportがCPU1を奪い合わない。 |
| DMA channels | HAL resource table | Critical | 不可 | privileged service API経由のみ | reservation map、active transfer、overrun/underrun | 直接公開なし | 静的予約を優先。動的割当はHAL内部でdeadline/priorityを見て行う。 |
| PIO state machines | HAL resource table | Critical | 不可 | privileged service API経由のみ | reservation map、loaded program、state machine status | 直接公開なし | backend用途ごとに固定割当を基本にする。PIO所有権をapp-visible capabilityにしない。 |
| SPI LCD bus | koto-gfx display service + HAL SPI backend | High | app surfaceとpresent requestのみ | launcher/menu/overlay/captureのためdisplay ownershipを取得可能 | flush timing、dirty rect list、SPI/DMA transfer state | surface作成、dirty rect、present | display serviceがbus利用を直列化する。system overlay/takeoverはapp presentより優先できる。 |
| PSRAM bus | koto-psram + memory manager | High | VM/runtime経由のallocationのみ | system pool/cacheを確保可能 | heap map、DMA-safe region map、fragmentation | app memory allocation、承認済みmapped asset read | PSRAM timingとbus設定はHAL所有。DMA-safe領域と通常領域を分離する。 |
| SD bus | koto-storage | High | app bundle readとsave sandboxのみ | install/delete/update/backup/migration | filesystem diagnostics、transaction log、write lock state | bundle read、save read/write/list | single writer policy。system install/updateはapp writeを一時停止できる。 |
| timers/alarms | kernel timer service | Critical | logical timerとmonotonic timeのみ | power、scheduler、service deadline用に利用 | hardware alarm map、ISR latency、missed deadline | monotonic time、sleep、timer event | hardware alarmは予約資源。app timerはinterrupt contextではなくeventとして配送する。 |
| audio output | koto-audio mixer | High | source作成とclip/stream/sequence enqueueのみ | audio focus、global volume、mute、device policy | mixer load、underrun count、backend state | clip再生、stream enqueue、sequence、stop、volume | mixerがbackendを所有する。過負荷時は低優先sourceをdegrade/dropする。 |
| input scan | koto-input | High | snapshot、event stream、text input、virtual gamepadのみ | global shortcut、launcher/menu focus、IME control | raw scan matrix、debounce state、repeat state | key snapshot、pressed/released/repeat、text input、gamepad | raw scan周期はOS固定。system shortcut/IMEがapp配送前にeventを消費・変換できる。 |
| UART/USB transport | transport manager / monitor service | High | 必要時のみvirtual channel | `koto run`、`koto install`、`koto monitor`、capture、logs | raw packet trace、profile stream、panic channel | app virtual channel if enabled | monitor priority。debug/install trafficは通常app channelをpause/disconnectできる。 |
| display framebuffer/surface | koto-gfx | High | logical app surfaceを所有。物理LCDは不可 | overlay、status bar、launcher/menu surface、capture | surface registry、damage visualization | surface create、draw/commit region、present | app surfaceはdisplay serviceがcomposite/flushする。system UIはforeground ownershipを取得できる。 |
| app storage/save area | koto-storage | High | per-app sandboxのみ | install/delete/migrate/backup | sandbox manifest、open handle、write transaction state | save read/write/list/delete | sandbox外アクセスは禁止。write lock取得後に書き、install/update中は遅延可能。 |
| power/suspend manager | power manager + kernel | Critical | status query、suspend request、keepalive | force suspend/resume、power policy、brightness/audio policy | forced transition test、wake reason、device quiesce state | power status、suspend request、keepalive | suspend中はpower managerが一時的にdevice ownershipを集約する。serviceはquiesceまたはpolicy上のvetoを返す。 |

## 3. CPU0/CPU1 Policy

CPU0はVM/app scheduler中心にする。KotoOS kernel loop、KotoVM scheduling、app lifecycle、timer event delivery、system service coordinationはCPU0を基準に動く。通常アプリコードは、将来明示的なworker abstractionを設計しない限り、CPU0上のVM sliceとして実行する。

CPU1はsystem work用に予約するが、初期版では所有者固定にする。複数serviceでCPU1を奪い合わない。

CPU1所有モデルの候補:

- Display-owned CPU1: SPI LCD flush、dirty rect conversion、surface compositionが支配的な場合に有効。
- Audio-owned CPU1: CPU制御audio mixerのdeadlineが厳しく、underrunが問題になる場合に有効。
- Storage/transport-owned CPU1: SD I/O、install、logging、monitor trafficをapp schedulingから隔離したい場合に有効。
- Shared service executor: 単一ownerが明確なqueue/deadline policyを持つ場合のみ許容する。

初期推奨は、CPU1を `system_service_core` という単一ownerに固定し、その内部で静的なadmission ruleを持つことである。音声がCPU制御でdeadlineを持つ場合はaudio deadlineを最優先にする。display flushとstorage処理はbounded taskとしてqueueに入れる。transport monitorはbest-effort workをpreemptできるが、audio deadlineは壊さない。

CPU1割当を変更する場合は、build-levelまたはboot-levelのpolicy変更として扱う。runtimeでdisplay、audio、storage、transportが独立にCPU1をclaimする設計にはしない。

## 4. Display Ownership

app-facingな表示対象はapp surfaceである。app surfaceはkoto-gfxが所有するlogical drawing targetであり、内部RAM、PSRAM、tile backing storeなど、実装に応じた配置を取れる。通常アプリは物理LCD転送経路を知らない。

display updateの流れ:

1. appがkoto-gfx APIまたはKotoVM hostcallでapp surfaceへ描画する。
2. appまたはkoto-gfxがdirty rectを記録する。
3. appがpresent requestを送る。
4. display serviceがdamageをmergeし、system overlay/status barを合成する。
5. display flush taskがSPI/DMA/PIO backendへ転送を依頼する。

SPI LCD bus、display DMA、PIO state machine、LCD command/data GPIO、reset/backlight GPIO、flush timingはdisplay serviceとHALが所有する。通常アプリはraw SPI transaction、raw framebuffer-to-LCD transfer、DMA channel予約を取得できない。

system overlay/status barはprivileged surface layerである。battery、transport、debug、modal prompt、status情報を表示できるが、foreground appへdisplay backend所有権を渡さない。

launcher/system menuはdisplay ownership takeoverを行える。takeover中はapp surfaceをfreeze、hide、composite下層化、または完全置換できる。復帰時はsurfaceが有効なら再表示し、無効ならappへrepaintを要求する。appはpresentを「可視LCDを所有した証拠」ではなく「表示要求」として扱う。

## 5. Audio Ownership

koto-audioはaudio mixerと物理audio backendを所有する。通常アプリが投入できるのはlogical sourceである。

- Clip: 短いmemory-resident sound。
- Stream: producer-fed PCMまたはencoded chunk。
- Sequence: note、event、compact playback instructionなどの時系列データ。

sourceはpriority、volume、loop state、completion callbackなどのmetadataとともにsource queueへ入る。mixerはactive sourceをbackend formatへ合成する。backendはPWM、PIO、DMA、timer IRQ、CPU-fed outputのいずれを使ってもよいが、その選択はkoto-audio/HAL内部に隠す。

underrun policy:

- backend clockは維持し、mixed dataがない場合はsilenceまたは安全なneutral sampleを出す。
- underrun countをdebug diagnosticsに記録する。
- high-priority UI/audio feedbackを守るため、低優先sourceをdegrade/dropする。
- 通常アプリにaudio backend上でbusy-waitさせない。

低優先sourceのdegradeには、短いbuffering、低sample rate化、mono化、frame skip、dropを含められる。system soundは通常app ambienceより高いpriorityを持てるが、user settingのmute/global volumeは常に適用する。

## 6. Input Ownership

koto-inputはraw scanを所有する。通常アプリはGPIO row/column、scan timing、debounce処理へアクセスしない。

input processing stages:

1. raw scanが物理keyboard/buttonを読む。
2. debounceとmatrix/state normalizationでkey snapshotを作る。
3. 前回snapshotとの差分からpressed/released eventを作る。
4. repeat policyに従ってrepeat eventを作る。
5. text contextが有効な場合、key eventをtext inputへ変換する。
6. virtual gamepad mappingがdirection/action inputを生成する。
7. IME integrationがkey eventを消費し、composed textを出力できる。

通常アプリはkey snapshot、pressed/released/repeat、text input、virtual gamepadを読む。system appはlauncher、menu、dialog、IME、global shortcutのためinput focusを取得できる。

IME integrationはraw scanより上、app text deliveryより下に置く。appが独自scanや独自repeat policyを実装しないとtext inputできない、という構造にはしない。

## 7. Storage Ownership

koto-storageはSD filesystemと物理block accessを所有する。通常アプリへ見せるstorage domainは次の2つに限定する。

- App bundle read: install済みapp bundle内のread-only file。
- Save sandbox: appごとのwritable data area。

system appはinstall、delete、update、backup、migration、repairを所有する。これらはglobal storage consistencyに関わるため、必要に応じてapp save writeを一時停止できる。

crash/power-loss safety:

- file replacementは可能な範囲でtemp/commitまたはjournal相当のtransaction policyを使う。
- save metadata更新は、途中電源断で無関係なapp dataを壊さない順序にする。
- low-power状態またはsuspend準備中は、長いwriteを遅延または拒否できる。
- app終了、uninstall、crash時はopen handleをclose、flush、またはinvalidateする。

write lock policy:

- storage write lockは同時に1 ownerだけが持つ。
- 通常app save writeはsystem install/delete/updateとemergency crash logより低優先にする。
- filesystem backendがconsistencyを保証できる場合、readは多くのwrite中にも継続できる。
- power managerはsuspend前にwrite quiesceを要求できる。

## 8. Transport/Debug Ownership

transport managerはUART/USB monitor pathを所有する。対象となるdevelopment operation:

- `koto run`
- `koto install`
- `koto monitor`
- log streaming
- capture
- profile
- panic/crash reporting

monitor priorityを持つ。debug/install/monitor trafficは、通常アプリのvirtual channelより優先する。将来、通常アプリにvirtual channelを提供する場合も、それはraw USB/UART endpointではなくframed logical streamである。system policyによりpause、rate-limit、revoke、disconnectできる。

debug buildだけが低レイヤ観測口を持つ。

- transport packet trace。
- log/capture/profile stream state。
- resource reservation table。
- panic-time forced output path。
- monitor counterとbandwidth。

release normal appはmonitor権限を持たず、install、run、debug sessionへ干渉できない。

## 9. Hostcall Boundary

通常アプリに公開するhostcall:

- CPU/time: `yield`、`sleep`、monotonic time、timer event registration。
- Display: app surface作成、dirty rect、present、logical size query。
- Audio: source作成、clip/stream/sequence enqueue、stop、app volume。
- Input: key snapshot、event poll、repeat preference、text input request、virtual gamepad。
- Storage: app bundle read、save sandbox read/write/list/delete。
- PSRAM/memory: VM/runtime policy経由のallocation。raw bus controlは不可。
- Power: power state query、suspend request、activity/keepalive hint。
- Transport: system policyで有効な場合のみapp virtual channel。

システムアプリだけに公開するhostcall:

- Display: foreground display ownership take/release、overlay、status bar、capture surface。
- Audio: audio focus、global volume/mute、system sound priority。
- Input: global shortcut registration、focus takeover、IME control。
- Storage: install、delete、update、migrate、backup、repair、app enumerate。
- Transport: run、install、monitor、capture、profile、log routing。
- Power: force suspend/resume、power policy、brightness policy、device quiesce。
- Scheduler: app launch/stop、foreground/background state、watchdog policy。

debug buildだけのhostcall:

- DMA、PIO、timers、CPU1 owner、bus、service queueのreservation table参照。
- display flush queue、dirty rect visualization、transfer timing参照。
- audio underrun counter、mixer load、backend state参照。
- input raw scan、debounce state参照。
- storage lock、open handle、transaction state参照。
- transport packet trace、monitor counter参照。
- forced diagnostic suspend/resume、panic capture、service reset。

将来予約:

- dynamic arbitration token API。
- multitasking向けper-app audio focus/display focus。
- per-app virtual USB channel。
- storage transaction API。
- power budget API。
- high precision profiling timer。
- optional external-device/near-field session API。

## 10. Implementation Roadmap

### Phase 1: static ownership table and no raw access

- CPU1、DMA、PIO、SPI、timers、SD、transport、audio backendのcompile-time resource ownership tableを定義する。
- hostcall boundaryで通常アプリのraw hardware accessを禁止する。
- 通常アプリAPIをlogical object操作に限定する。
- 各physical resourceのowner moduleを文書化する。

### Phase 2: display/audio/input/storage service boundary

- koto-gfxでsurface、dirty rect、present、display flush ownershipを実装する。
- koto-audioでmixer/source/backendを分離する。
- koto-inputでscan/snapshot/event/text/gamepadを分離する。
- koto-storageでbundle readとsave sandbox write policyを実装する。
- power managerが各serviceをquiesceできるようにする。

### Phase 3: resource diagnostics

- DMA、PIO、timers、CPU1、SPI、SD、transport、service queueのdebug-only reservation viewを追加する。
- display flush latency、audio underrun、input scan timing、storage lock wait、monitor bandwidthのcounterを追加する。
- diagnosticsを通常アプリABIへ入れない。

### Phase 4: system app permissions

- launcher、system menu、installer、monitor、settings、IME、debug tool向けの明示permissionを導入する。
- display takeover、input focus takeover、install/delete、monitor operation、power policyをsystem app permissionでgateする。
- busy resourceに対するsystem app requestの失敗動作を定義する。

### Phase 5: dynamic arbitration if needed

- static ownershipが制約になった場合だけdynamic arbitrationを追加する。
- arbitrationはsystem service/HAL内部に閉じ、通常アプリへraw hardware leaseを公開しない。
- priority、deadline、revocation、diagnosticsの規則を明示する。
- logical hostcallを安定させ、通常アプリ互換性を保つ。

## 11. Design Conclusion

KotoOSは、P/ECE Resource Reservation Modelから得られる教訓をさらに強める。アプリは意図を表明し、OSが物理実行を所有する。DisplayはsurfaceとpresentでありSPIではない。AudioはsourceとmixerでありPWM/DMAではない。Inputはsnapshot/eventでありGPIO scanではない。Storageはbundle/save sandboxでありSD sectorではない。Transportはmonitorまたはvirtual channelでありraw endpoint ownershipではない。

PicoCalc/RP2040では、CPU core、DMA channel、PIO state machine、SPI bus、PSRAM、SD、audio timingが相互に干渉しやすい。初期KotoOSは固定所有権と明示service boundaryを優先する。dynamic sharingは後から追加できるが、通常アプリへのraw hardware leasingをpublic app modelにしてはいけない。
