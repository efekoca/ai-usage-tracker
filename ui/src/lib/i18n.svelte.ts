// Turkish / English strings and locale-aware formatting. The language follows the system
// unless the user picks one in Settings.

type Dict = Record<string, string>

const tr: Dict = {
  'app.name': 'AI Usage Tracker',
  'nav.overview': 'Genel bakış',
  'nav.daily': 'Günlük',
  'nav.breakdown': 'Kırılımlar',
  'nav.limits': 'Limitler',
  'nav.projects': 'Projeler',
  'nav.sources': 'Kaynaklar',
  'nav.settings': 'Ayarlar',
  'nav.section.usage': 'Kullanım',
  'nav.section.manage': 'Yönet',

  'period.today': 'Bugün',
  'period.days7': '7G',
  'period.month1': '1A',
  'period.months3': '3A',
  'period.months6': '6A',
  'period.year1': '1Y',
  'period.all': 'Tümü',
  'period.custom': 'Özel',
  'period.label': 'Dönem',
  'period.from': 'Başlangıç',
  'period.to': 'Bitiş',
  'period.apply': 'Uygula',

  'tool.claude_code': 'Claude Code',
  'tool.codex': 'Codex',
  'tool.claude_desktop': 'Claude masaüstü',
  'provider.anthropic': 'Claude',
  'provider.openai': 'Codex',
  'client.cowork': 'Cowork',
  'client.claude-desktop': 'Claude masaüstü (Code)',
  'client.cli': 'Terminal (CLI)',
  'client.claude-vscode': 'VS Code',
  'client.unknown': 'Bilinmiyor',

  'source.claude_code': 'Claude Code',
  'source.cowork': 'Cowork oturumları',
  'source.claude_desktop': 'Claude masaüstü — plan kullanımı',
  'source.codex': 'Codex (CLI ve masaüstü)',
  'source.chatgpt_desktop': 'ChatGPT masaüstü',
  'source.desc.claude_code': 'Oturum kayıtlarındaki token sayıları (kesin).',
  'source.desc.cowork': 'Cowork oturumlarının token sayıları ve limit yüzdeleri (kesin).',
  'source.desc.claude_desktop': 'Uygulamanın kaydettiği 5 saatlik ve haftalık plan kullanım yüzdesi (kesin). Sohbet token\'ları yerelde tutulmaz.',
  'source.desc.codex': 'Rollout kayıtlarındaki token sayıları ve plan limit yüzdeleri (kesin).',
  'source.desc.chatgpt_desktop': 'Sohbetler sunucuda tutulur; yerelde okunabilecek kullanım verisi yok.',
  'source.found': 'Bulundu',
  'source.notFound': 'Bulunamadı',
  'source.unsupported': 'Veri yok',
  'source.files': '{n} dosya',

  'acc.exact': 'Kesin',
  'acc.estimated': 'Tahmini',
  'acc.captured': 'Yakalanan',
  'acc.exact.help': 'Aracın kendi kayıt dosyasına yazdığı sayı.',
  'acc.estimated.help': 'Bu uygulamanın hesapladığı değer; gerçek değer farklı olabilir.',
  'acc.captured.help': 'Bu uygulamanın kendi canlı yakalamasıyla kaydedildi.',

  'metric.tokens': 'Token',
  'metric.cost': 'API eşdeğeri',
  'metric.apiEq': 'API eşdeğeri maliyet',
  'metric.apiEq.help': 'Bu kullanım API üzerinden yapılsaydı resmi liste fiyatlarıyla tutacak tutar. Abonelikle ödediğiniz tutar değildir.',
  'metric.events': 'İstek',
  'metric.input': 'Girdi',
  'metric.output': 'Çıktı',
  'metric.cacheRead': 'Önbellekten okuma',
  'metric.cacheWrite': 'Önbelleğe yazma',
  'metric.reasoning': 'Akıl yürütme',
  'metric.reasoning.help': 'Çıktı token\'larının içinde yer alır.',
  'metric.total': 'Toplam',

  'overview.title': 'Genel bakış',
  'overview.totalTokens': 'Toplam token',
  'overview.cost': 'API eşdeğeri maliyet',
  'overview.avgDaily': 'Günlük ortalama',
  'overview.activeDays': 'Aktif gün',
  'overview.vsPrev': 'önceki döneme göre',
  'overview.trend': 'Trend',
  'overview.byTool': 'Araçlara göre',
  'overview.composition': 'Token dağılımı',
  'overview.topModels': 'En çok kullanılan modeller',
  'overview.limitsNow': 'Şu anki limitler',
  'overview.peakDay': 'En yoğun gün',
  'overview.peakHour': 'En yoğun saat',
  'overview.unpriced': '{n} model için fiyat tanımlı değil; bu kullanım maliyete dahil edilmedi: {models}',
  'overview.unpricedAction': 'Fiyatları düzenle',
  'overview.excludesAux': 'Claude Code kayıtları yalnızca ana model çağrılarını içerir; arka plandaki yardımcı çağrılar (ör. web araması özetleme) dahil değildir.',

  'daily.title': 'Günlük kayıt',
  'daily.calendar': 'Takvim',
  'daily.table': 'Tablo',
  'daily.date': 'Tarih',
  'daily.noData': 'Bu gün kullanım yok',

  'breakdown.title': 'Kırılımlar',
  'breakdown.models': 'Modeller',
  'breakdown.tools': 'Araçlar ve istemciler',
  'breakdown.hours': 'Gün ve saat yoğunluğu',
  'breakdown.categories': 'Token kategorileri',
  'breakdown.share': 'Pay',

  'limits.title': 'Limitler',
  'limits.window.five_hour': '5 saatlik pencere',
  'limits.window.seven_day': 'Haftalık',
  'limits.window.thirty_day': '30 günlük',
  'limits.window.one_day': 'Günlük',
  'limits.window.seven_day_opus': 'Haftalık (Opus)',
  'limits.window.seven_day_sonnet': 'Haftalık (Sonnet)',
  'limits.used': '%{pct} kullanıldı',
  'limits.resetsIn': '{t} sonra sıfırlanır',
  'limits.resetsAt': 'Sıfırlanma: {t}',
  'limits.observed': '{t} önce okundu',
  'limits.state.reset': 'Pencere son okumadan sonra sıfırlandı; güncel değer bilinmiyor.',
  'limits.state.stale': 'Okuma eski; güncel değer bilinmiyor.',
  'limits.source.claude_plan_history': 'Claude masaüstü uygulaması',
  'limits.source.cowork_audit': 'Cowork oturumu',
  'limits.source.claude_code_jsonl': 'Claude Code (limit uyarısı)',
  'limits.source.codex_rollout': 'Codex oturumu',
  'limits.source.user_threshold': 'Sizin tanımladığınız bütçe',
  'limits.windowUsage': 'Bu penceredeki kullanım (yerel kayıtlar)',
  'limits.byProject': 'Projelere göre tahmini pay',
  'limits.byProject.help': 'Limit hesap genelidir. Proje payı, penceredeki API eşdeğeri maliyetin oranından hesaplanır.',
  'limits.none': 'Henüz limit bilgisi yok. Araçları kullandıkça burada görünür; isterseniz aşağıdan kendi bütçenizi tanımlayın.',
  'limits.thresholds': 'Kendi bütçeleriniz',
  'limits.thresholds.help': 'Gerçek limit yüzdesi okunamadığında kullanılır ve “Tahmini” olarak gösterilir. Resmi limitler sayı olarak yayınlanmıyor.',
  'limits.addThreshold': 'Bütçe ekle',
  'limits.plan': 'Plan',
  'limits.planHint': 'Plan seçimi limit göstergelerini düzenler; sayısal limitler sağlayıcılar tarafından yayınlanmaz.',
  'limits.status.ok': 'Normal',
  'limits.status.warn': 'Yaklaşıyor',
  'limits.status.high': 'Yüksek',
  'limits.status.full': 'Doldu',
  'limits.status.unknown': 'Bilinmiyor',

  'projects.title': 'Projeler',
  'projects.hide': 'Adı gizle',
  'projects.hidden': 'Gizli proje',
  'projects.noProject': 'Proje yok',
  'projects.help': 'Proje adları yalnızca bu bilgisayarda tutulur. Gizlenen projelerin verisi sayılmaya devam eder, adı görünmez ve dışa aktarılmaz.',
  'projects.filter': 'Bu projeye göre filtrele',

  'sources.title': 'Kaynaklar',
  'sources.help': 'Uygulama yalnızca açık olan kaynakların kullanım kayıtlarını okur. İçerik (prompt/yanıt) hiçbir zaman saklanmaz.',
  'sources.rescan': 'Şimdi tara',
  'sources.scanning': 'Taranıyor… {done}/{total}',
  'sources.lastScan': 'Son tarama: {t}',
  'sources.extra': 'Ek klasörler',
  'sources.extra.claude': 'Claude Code yapılandırma klasörü',
  'sources.extra.codex': 'Codex klasörü (CODEX_HOME)',
  'sources.addFolder': 'Klasör ekle',
  'sources.warnings': 'Okunamayan veya tanınmayan kayıtlar',
  'sources.warnings.help': 'Bir aracın kayıt biçimi değişmiş olabilir. Okunabilen kısım kullanılır; bilinmeyen satırlar atlanır.',
  'sources.retention': 'Claude Code CLI eski oturum kayıtlarını varsayılan olarak 30 gün sonra siler. Bu uygulama içe aldığı kayıtları kendi arşivinde saklar; geçmişin kaybolmaması için uygulamayı açık tutmanız yeterli.',

  'settings.title': 'Ayarlar',
  'settings.general': 'Genel',
  'settings.language': 'Dil',
  'settings.language.system': 'Sistem',
  'settings.theme': 'Görünüm',
  'settings.theme.system': 'Sistem',
  'settings.theme.light': 'Açık',
  'settings.theme.dark': 'Koyu',
  'settings.autostart': 'Windows ile başlat',
  'settings.autostart.help': 'Oturum açınca widget ile birlikte arka planda başlar.',
  'settings.primaryMetric': 'Öncelikli gösterge',
  'settings.currency': 'Para birimi',
  'settings.fxRate': '1 USD =',
  'settings.currency.help': 'Maliyetler USD üzerinden hesaplanır. Başka para birimi seçerseniz kuru kendiniz girersiniz; internetten kur çekilmez.',
  'settings.privacy': 'Gizlilik',
  'settings.hideProjects': 'Tüm proje adlarını gizle',
  'settings.privacy.text': 'Okunan: araçların kendi kayıt dosyalarındaki zaman, model, proje klasörü ve token sayıları. Saklanan: yalnızca bu sayılar ve meta veriler (%LOCALAPPDATA%\\AIUsageTracker). Asla saklanmayan: prompt ve yanıt içerikleri, kimlik bilgileri. Hiçbir veri bilgisayarınızdan gönderilmez; telemetri yoktur.',
  'settings.widget': 'Widget',
  'settings.widget.show': 'Widget\'ı göster',
  'settings.widget.opacity': 'Saydamlık',
  'settings.widget.size': 'Boyut',
  'settings.widget.autohide': 'Tam ekran uygulamalarda gizle',
  'settings.size.s': 'Küçük',
  'settings.size.m': 'Orta',
  'settings.size.l': 'Büyük',
  'settings.plans': 'Planlar',
  'settings.pricing': 'Fiyatlar',
  'settings.pricing.help': 'USD / 1 milyon token. Fiyatlar resmi sağlayıcı sayfalarından alınmıştır; düzenleyebilir veya varsayılana dönebilirsiniz. Tanımsız modeller maliyete eklenmez.',
  'settings.pricing.updated': 'Son güncelleme: {d}',
  'settings.pricing.origin.bundled': 'Uygulamayla gelen fiyatlar',
  'settings.pricing.origin.user': 'Sizin düzenlediğiniz fiyatlar',
  'settings.pricing.reset': 'Varsayılana dön',
  'settings.pricing.save': 'Fiyatları kaydet',
  'settings.pricing.alias': 'Fiyatsız modeli başka bir modelin fiyatıyla hesapla',
  'settings.pricing.aliasAs': 'şu modelin fiyatıyla:',
  'settings.pricing.sources': 'Kaynaklar',
  'settings.data': 'Veriler',
  'settings.data.export': 'Dışa aktar',
  'settings.data.exportEvents': 'İstek bazında',
  'settings.data.exportDaily': 'Günlük özet',
  'settings.data.backup': 'Veritabanını yedekle',
  'settings.data.import': 'Yedekten içe aktar',
  'settings.data.import.help': 'Yedekteki kayıtlar mevcut kayıtlarla birleştirilir; aynı kayıt iki kez sayılmaz.',
  'settings.data.openFolder': 'Veri klasörünü aç',
  'settings.data.wipe': 'Tüm verilerimi sil',
  'settings.data.wipe.confirm': 'Bu uygulamanın arşivindeki tüm kullanım kayıtları silinecek. Araçların kendi kayıt dosyalarına dokunulmaz. Ardından kurulum ekranı açılır ve siz onaylayana kadar hiçbir şey yeniden okunmaz. Devam edilsin mi?',
  'settings.data.done': 'Tamamlandı',
  'settings.data.exported': '{n} satır dışa aktarıldı',
  'settings.data.imported': '{n} kayıt birleştirildi',
  'settings.network': 'Ağ',
  'settings.network.text': 'Uygulama varsayılan olarak hiçbir ağ bağlantısı kurmaz.',
  'settings.about': 'Hakkında',
  'settings.version': 'Sürüm {v}',
  'settings.quit': 'Uygulamadan çık',

  'onb.welcome': 'AI kullanımınız, tek yerde.',
  'onb.lead': 'Bilgisayarınızdaki yapay zekâ araçlarının zaten tuttuğu kayıtları okuyup token, API eşdeğeri maliyet ve plan limitlerini gösterir. İçerik okunmaz, hiçbir veri dışarı gönderilmez.',
  'onb.detected': 'Bu bilgisayarda bulunanlar',
  'onb.none': 'Desteklenen bir araç bulunamadı. Claude Code, Claude masaüstü (Cowork) veya Codex kurduğunuzda uygulama onları otomatik algılar; özel bir klasör kullanıyorsanız Kaynaklar\'dan ekleyebilirsiniz.',
  'onb.plans': 'Planınız (isteğe bağlı)',
  'onb.plan.none': 'Bilmiyorum / seçme',
  'onb.start': 'Başla',
  'onb.privacy': 'Ne okunur, ne saklanır?',

  'common.loading': 'Yükleniyor…',
  'common.empty': 'Bu dönemde kullanım yok.',
  'common.save': 'Kaydet',
  'common.cancel': 'Vazgeç',
  'common.remove': 'Kaldır',
  'common.on': 'Açık',
  'common.off': 'Kapalı',
  'common.other': 'Diğer',
  'common.all': 'Tümü',
  'common.table': 'Tablo görünümü',
  'common.chart': 'Grafik görünümü',
  'common.filters': 'Filtreler',
  'common.clear': 'Temizle',
  'common.model': 'Model',
  'common.project': 'Proje',
  'common.tool': 'Araç',
  'common.ago.now': 'az',
  'common.minutes': '{n} dk',
  'common.hours': '{n} sa',
  'common.days': '{n} gün',
  'common.error': 'Bir hata oluştu: {e}',

  'widget.today': 'Bugün',
  'widget.noSources': 'Kaynak seçilmedi',
  'widget.open': 'Paneli aç',
  'widget.5h': '5 sa',
  'widget.week': 'Hafta',
}

const en: Dict = {
  'app.name': 'AI Usage Tracker',
  'nav.overview': 'Overview',
  'nav.daily': 'Daily',
  'nav.breakdown': 'Breakdown',
  'nav.limits': 'Limits',
  'nav.projects': 'Projects',
  'nav.sources': 'Sources',
  'nav.settings': 'Settings',
  'nav.section.usage': 'Usage',
  'nav.section.manage': 'Manage',

  'period.today': 'Today',
  'period.days7': '7D',
  'period.month1': '1M',
  'period.months3': '3M',
  'period.months6': '6M',
  'period.year1': '1Y',
  'period.all': 'All',
  'period.custom': 'Custom',
  'period.label': 'Period',
  'period.from': 'From',
  'period.to': 'To',
  'period.apply': 'Apply',

  'tool.claude_code': 'Claude Code',
  'tool.codex': 'Codex',
  'tool.claude_desktop': 'Claude desktop',
  'provider.anthropic': 'Claude',
  'provider.openai': 'Codex',
  'client.cowork': 'Cowork',
  'client.claude-desktop': 'Claude desktop (Code)',
  'client.cli': 'Terminal (CLI)',
  'client.claude-vscode': 'VS Code',
  'client.unknown': 'Unknown',

  'source.claude_code': 'Claude Code',
  'source.cowork': 'Cowork sessions',
  'source.claude_desktop': 'Claude desktop — plan usage',
  'source.codex': 'Codex (CLI and desktop)',
  'source.chatgpt_desktop': 'ChatGPT desktop',
  'source.desc.claude_code': 'Token counts from session logs (exact).',
  'source.desc.cowork': 'Token counts and limit percentages from Cowork sessions (exact).',
  'source.desc.claude_desktop': 'Five-hour and weekly plan usage the app records (exact). Chat tokens are not stored locally.',
  'source.desc.codex': 'Token counts and plan-limit percentages from rollout logs (exact).',
  'source.desc.chatgpt_desktop': 'Chats live on the server; there is no local usage data to read.',
  'source.found': 'Found',
  'source.notFound': 'Not found',
  'source.unsupported': 'No data',
  'source.files': '{n} files',

  'acc.exact': 'Exact',
  'acc.estimated': 'Estimated',
  'acc.captured': 'Captured',
  'acc.exact.help': 'A number the tool wrote into its own log.',
  'acc.estimated.help': 'Computed by this app; the real value may differ.',
  'acc.captured.help': 'Recorded by this app’s own live capture.',

  'metric.tokens': 'Tokens',
  'metric.cost': 'API equivalent',
  'metric.apiEq': 'API-equivalent cost',
  'metric.apiEq.help': 'What this usage would cost at official API list prices. Not what you pay for a subscription.',
  'metric.events': 'Requests',
  'metric.input': 'Input',
  'metric.output': 'Output',
  'metric.cacheRead': 'Cache read',
  'metric.cacheWrite': 'Cache write',
  'metric.reasoning': 'Reasoning',
  'metric.reasoning.help': 'Included in output tokens.',
  'metric.total': 'Total',

  'overview.title': 'Overview',
  'overview.totalTokens': 'Total tokens',
  'overview.cost': 'API-equivalent cost',
  'overview.avgDaily': 'Daily average',
  'overview.activeDays': 'Active days',
  'overview.vsPrev': 'vs previous period',
  'overview.trend': 'Trend',
  'overview.byTool': 'By tool',
  'overview.composition': 'Token mix',
  'overview.topModels': 'Top models',
  'overview.limitsNow': 'Limits right now',
  'overview.peakDay': 'Busiest day',
  'overview.peakHour': 'Busiest hour',
  'overview.unpriced': '{n} model(s) have no price, so their usage is not in the cost: {models}',
  'overview.unpricedAction': 'Edit prices',
  'overview.excludesAux': 'Claude Code logs contain the main model calls only; background helper calls (e.g. web-search summarisation) are not included.',

  'daily.title': 'Daily log',
  'daily.calendar': 'Calendar',
  'daily.table': 'Table',
  'daily.date': 'Date',
  'daily.noData': 'No usage on this day',

  'breakdown.title': 'Breakdown',
  'breakdown.models': 'Models',
  'breakdown.tools': 'Tools and clients',
  'breakdown.hours': 'Day and hour intensity',
  'breakdown.categories': 'Token categories',
  'breakdown.share': 'Share',

  'limits.title': 'Limits',
  'limits.window.five_hour': '5-hour window',
  'limits.window.seven_day': 'Weekly',
  'limits.window.thirty_day': '30-day',
  'limits.window.one_day': 'Daily',
  'limits.window.seven_day_opus': 'Weekly (Opus)',
  'limits.window.seven_day_sonnet': 'Weekly (Sonnet)',
  'limits.used': '{pct}% used',
  'limits.resetsIn': 'Resets in {t}',
  'limits.resetsAt': 'Resets {t}',
  'limits.observed': 'Read {t} ago',
  'limits.state.reset': 'The window has reset since the last reading; current use is unknown.',
  'limits.state.stale': 'Reading is old; current use is unknown.',
  'limits.source.claude_plan_history': 'Claude desktop app',
  'limits.source.cowork_audit': 'Cowork session',
  'limits.source.claude_code_jsonl': 'Claude Code (limit notice)',
  'limits.source.codex_rollout': 'Codex session',
  'limits.source.user_threshold': 'Your own budget',
  'limits.windowUsage': 'Usage in this window (local logs)',
  'limits.byProject': 'Estimated share by project',
  'limits.byProject.help': 'Limits are account-wide. A project’s share is its fraction of the window’s API-equivalent cost.',
  'limits.none': 'No limit readings yet. They appear as you use the tools; you can also set your own budget below.',
  'limits.thresholds': 'Your budgets',
  'limits.thresholds.help': 'Used only when no real limit reading exists, and shown as “Estimated”. Official limits are not published as numbers.',
  'limits.addThreshold': 'Add budget',
  'limits.plan': 'Plan',
  'limits.planHint': 'Your plan shapes the limit display; providers do not publish numeric limits.',
  'limits.status.ok': 'Normal',
  'limits.status.warn': 'Getting close',
  'limits.status.high': 'High',
  'limits.status.full': 'Reached',
  'limits.status.unknown': 'Unknown',

  'projects.title': 'Projects',
  'projects.hide': 'Hide name',
  'projects.hidden': 'Hidden project',
  'projects.noProject': 'No project',
  'projects.help': 'Project names stay on this computer. Hidden projects still count, but their name is not shown or exported.',
  'projects.filter': 'Filter by this project',

  'sources.title': 'Sources',
  'sources.help': 'Only enabled sources are read. Content (prompts/responses) is never stored.',
  'sources.rescan': 'Rescan now',
  'sources.scanning': 'Scanning… {done}/{total}',
  'sources.lastScan': 'Last scan: {t}',
  'sources.extra': 'Extra folders',
  'sources.extra.claude': 'Claude Code config folder',
  'sources.extra.codex': 'Codex folder (CODEX_HOME)',
  'sources.addFolder': 'Add folder',
  'sources.warnings': 'Unreadable or unrecognised logs',
  'sources.warnings.help': 'A tool may have changed its log format. Readable parts are used; unknown lines are skipped.',
  'sources.retention': 'The Claude Code CLI deletes old session logs after 30 days by default. This app keeps everything it has imported in its own archive; keeping the app running is enough to preserve history.',

  'settings.title': 'Settings',
  'settings.general': 'General',
  'settings.language': 'Language',
  'settings.language.system': 'System',
  'settings.theme': 'Appearance',
  'settings.theme.system': 'System',
  'settings.theme.light': 'Light',
  'settings.theme.dark': 'Dark',
  'settings.autostart': 'Start with Windows',
  'settings.autostart.help': 'Starts in the background with the widget when you sign in.',
  'settings.primaryMetric': 'Primary metric',
  'settings.currency': 'Currency',
  'settings.fxRate': '1 USD =',
  'settings.currency.help': 'Costs are computed in USD. For another currency you enter the rate yourself; nothing is fetched online.',
  'settings.privacy': 'Privacy',
  'settings.hideProjects': 'Hide all project names',
  'settings.privacy.text': 'Read: timestamps, model, project folder and token counts from the tools’ own log files. Stored: only those numbers and metadata (%LOCALAPPDATA%\\AIUsageTracker). Never stored: prompt or response content, credentials. Nothing leaves your computer; there is no telemetry.',
  'settings.widget': 'Widget',
  'settings.widget.show': 'Show widget',
  'settings.widget.opacity': 'Opacity',
  'settings.widget.size': 'Size',
  'settings.widget.autohide': 'Hide during full-screen apps',
  'settings.size.s': 'Small',
  'settings.size.m': 'Medium',
  'settings.size.l': 'Large',
  'settings.plans': 'Plans',
  'settings.pricing': 'Prices',
  'settings.pricing.help': 'USD per 1M tokens, taken from official provider pages. Edit them or return to the defaults. Models without a price are left out of costs.',
  'settings.pricing.updated': 'Last updated: {d}',
  'settings.pricing.origin.bundled': 'Prices shipped with the app',
  'settings.pricing.origin.user': 'Your edited prices',
  'settings.pricing.reset': 'Restore defaults',
  'settings.pricing.save': 'Save prices',
  'settings.pricing.alias': 'Price an unpriced model like another model',
  'settings.pricing.aliasAs': 'priced as:',
  'settings.pricing.sources': 'Sources',
  'settings.data': 'Data',
  'settings.data.export': 'Export',
  'settings.data.exportEvents': 'Per request',
  'settings.data.exportDaily': 'Daily summary',
  'settings.data.backup': 'Back up database',
  'settings.data.import': 'Import from backup',
  'settings.data.import.help': 'Records from the backup are merged; the same record is never counted twice.',
  'settings.data.openFolder': 'Open data folder',
  'settings.data.wipe': 'Delete all my data',
  'settings.data.wipe.confirm': 'All usage records in this app’s archive will be deleted. The tools’ own log files are not touched. Setup will open again and nothing is re-read until you confirm. Continue?',
  'settings.data.done': 'Done',
  'settings.data.exported': 'Exported {n} rows',
  'settings.data.imported': 'Merged {n} records',
  'settings.network': 'Network',
  'settings.network.text': 'The app makes no network connections by default.',
  'settings.about': 'About',
  'settings.version': 'Version {v}',
  'settings.quit': 'Quit app',

  'onb.welcome': 'Your AI usage, in one place.',
  'onb.lead': 'Reads the logs your AI tools already keep on this computer and shows tokens, API-equivalent cost and plan limits. Content is never read and nothing is sent anywhere.',
  'onb.detected': 'Found on this computer',
  'onb.none': 'No supported tool was found. When you install Claude Code, the Claude desktop app (Cowork) or Codex, it will be detected automatically; custom folders can be added under Sources.',
  'onb.plans': 'Your plans (optional)',
  'onb.plan.none': 'Not sure / skip',
  'onb.start': 'Get started',
  'onb.privacy': 'What is read and stored?',

  'common.loading': 'Loading…',
  'common.empty': 'No usage in this period.',
  'common.save': 'Save',
  'common.cancel': 'Cancel',
  'common.remove': 'Remove',
  'common.on': 'On',
  'common.off': 'Off',
  'common.other': 'Other',
  'common.all': 'All',
  'common.table': 'Table view',
  'common.chart': 'Chart view',
  'common.filters': 'Filters',
  'common.clear': 'Clear',
  'common.model': 'Model',
  'common.project': 'Project',
  'common.tool': 'Tool',
  'common.ago.now': 'moments',
  'common.minutes': '{n} min',
  'common.hours': '{n} h',
  'common.days': '{n} d',
  'common.error': 'Something went wrong: {e}',

  'widget.today': 'Today',
  'widget.noSources': 'No sources selected',
  'widget.open': 'Open dashboard',
  'widget.5h': '5 h',
  'widget.week': 'Week',
}

const dicts: Record<string, Dict> = { tr, en }

export const i18n = $state({ lang: 'en' as 'tr' | 'en', currency: 'USD', fx: 1 })

export function resolveLang(pref: string): 'tr' | 'en' {
  if (pref === 'tr' || pref === 'en') return pref
  const sys = (navigator.languages?.[0] || navigator.language || 'en').toLowerCase()
  return sys.startsWith('tr') ? 'tr' : 'en'
}

export function t(key: string, params?: Record<string, string | number>): string {
  let s = dicts[i18n.lang][key] ?? dicts.en[key] ?? key
  if (params) for (const [k, v] of Object.entries(params)) s = s.replaceAll(`{${k}}`, String(v))
  return s
}

export function has(key: string): boolean {
  return key in dicts[i18n.lang] || key in dicts.en
}

const locale = () => (i18n.lang === 'tr' ? 'tr-TR' : navigator.language?.startsWith('en') ? navigator.language : 'en-US')

export function fmtInt(n: number): string {
  return new Intl.NumberFormat(locale(), { maximumFractionDigits: 0 }).format(n)
}

export function fmtCompact(n: number): string {
  if (Math.abs(n) < 10_000) return fmtInt(n)
  return new Intl.NumberFormat(locale(), { notation: 'compact', maximumFractionDigits: 1 }).format(n)
}

/** Percentage with the locale's sign placement (tr: "%46", en: "46%"); `n` is 0–100. */
export function fmtPct(n: number, digits = 0): string {
  return new Intl.NumberFormat(locale(), { style: 'percent', maximumFractionDigits: digits, minimumFractionDigits: digits }).format(n / 100)
}

/** Plain decimal number without a unit. */
export function fmtDec(n: number, digits = 0): string {
  return new Intl.NumberFormat(locale(), { maximumFractionDigits: digits, minimumFractionDigits: digits }).format(n)
}

/** USD amount converted with the user's manual rate. */
export function fmtMoney(usd: number, opts: { compact?: boolean } = {}): string {
  const v = usd * (i18n.currency === 'USD' ? 1 : i18n.fx)
  const digits = Math.abs(v) >= 100 ? 0 : Math.abs(v) >= 1 ? 2 : Math.abs(v) >= 0.01 ? 2 : 4
  return new Intl.NumberFormat(locale(), {
    style: 'currency',
    currency: i18n.currency || 'USD',
    notation: opts.compact && Math.abs(v) >= 10_000 ? 'compact' : 'standard',
    maximumFractionDigits: digits,
    minimumFractionDigits: Math.min(digits, 2),
  }).format(v)
}

export function fmtDate(d: string | number | Date, style: 'short' | 'medium' | 'long' | 'weekday' = 'medium'): string {
  const date = typeof d === 'string' ? new Date(d + (d.length === 10 ? 'T00:00:00' : '')) : new Date(d)
  const o: Intl.DateTimeFormatOptions =
    style === 'short'
      ? { day: 'numeric', month: 'short' }
      : style === 'long'
        ? { day: 'numeric', month: 'long', year: 'numeric', weekday: 'long' }
        : style === 'weekday'
          ? { weekday: 'short' }
          : { day: 'numeric', month: 'short', year: 'numeric' }
  return new Intl.DateTimeFormat(locale(), o).format(date)
}

export function fmtTime(ms: number): string {
  return new Intl.DateTimeFormat(locale(), { hour: '2-digit', minute: '2-digit', weekday: 'short' }).format(new Date(ms))
}

export function fmtHour(h: number): string {
  return new Intl.DateTimeFormat(locale(), { hour: '2-digit', minute: '2-digit' }).format(new Date(2000, 0, 1, h))
}

export function fmtDuration(ms: number): string {
  const m = Math.max(0, Math.round(ms / 60000))
  if (m < 1) return t('common.ago.now')
  if (m < 60) return t('common.minutes', { n: m })
  const h = Math.floor(m / 60)
  if (h < 48) return t('common.hours', { n: h }) + (h < 10 && m % 60 ? ' ' + t('common.minutes', { n: m % 60 }) : '')
  return t('common.days', { n: Math.round(h / 24) })
}

export function weekdayNames(): string[] {
  // Monday first, matching the core heatmap
  return [0, 1, 2, 3, 4, 5, 6].map((i) => new Intl.DateTimeFormat(locale(), { weekday: 'short' }).format(new Date(2024, 0, 1 + i)))
}

export function toolLabel(tool: string): string {
  return has(`tool.${tool}`) ? t(`tool.${tool}`) : tool
}

export function clientLabel(key: string, label: string): string {
  return has(`client.${label}`) ? t(`client.${label}`) : label
}

export function windowLabel(w: string): string {
  return has(`limits.window.${w}`) ? t(`limits.window.${w}`) : w
}

export function fmtMonth(d: string): string {
  return new Intl.DateTimeFormat(locale(), { month: 'short' }).format(new Date(d + 'T00:00:00'))
}
