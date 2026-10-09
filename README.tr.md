![AI Usage Tracker](src-tauri/icons/128x128.png)

# AI Usage Tracker

**Claude Code, Codex ve Antigravity kullanımınızı, API eşdeğeri maliyetinizi ve plan limitlerinizi tek yerden izleyin.**

AI Usage Tracker, Windows, macOS ve Linux'ta çalışan bir uygulamadır. Yapay zekâ araçlarınızın bilgisayarınıza zaten yazdığı kullanım kayıtlarını okur ve size anlaşılır bir panelde sunar. Ne kadar token harcadığınızı, bu kullanımın API fiyatlarıyla ne tutacağını ve limitlerinizin ne zaman dolacağını görürsünüz. Hesap açmanız gerekmez; verileriniz bulutta değil, kendi bilgisayarınızda kalır.

[![Windows, macOS ve Linux için indir](https://img.shields.io/badge/%C4%B0ndir-Windows%20%C2%B7%20macOS%20%C2%B7%20Linux-2563eb?style=for-the-badge)](../../releases/latest)

![Windows 10 | 11](https://img.shields.io/badge/Windows-10%20%7C%2011-0078D4?style=flat-square&logo=windows&logoColor=white) ![macOS 11+](https://img.shields.io/badge/macOS-11%2B-000000?style=flat-square&logo=apple&logoColor=white) ![Linux x86_64](https://img.shields.io/badge/Linux-x86__64-FCC624?style=flat-square&logo=linux&logoColor=black) [![Lisans: MIT](https://img.shields.io/badge/lisans-MIT-64748b?style=flat-square)](LICENSE)

[Özellikler](#özellikler) · [Kurulum](#kurulum) · [Gizlilik](#gizlilik) · [Nasıl çalışır](#nasıl-çalışır) · [Kaynaktan derleme](#kaynaktan-derleme) · [English](README.md)

![Genel bakış ekranı: 30 günlük trend, token dağılımı ve güncel limitler; yanında bugünkü token sayısını ve limit halkalarını gösteren masaüstü widget'ı](assets/readme/hero-tr.webp)

- **Verileriniz sizde kalır.** Prompt ve yanıtlarınız uygulamanın arşivine hiçbir zaman kaydedilmez. Uygulama yalnızca sayıları ve meta verileri bilgisayarınızdaki bir dosyada tutar ve hiçbir yere göndermez.
- **Her sayının nereden geldiğini bilirsiniz.** Araçtan doğrudan okunan değerler **Kesin**, uygulamanın hesapladıkları **Tahmini**, canlı okunanlar **Yakalanan** etiketiyle gösterilir. Hiçbir değer sessizce tahmin edilmez.
- **Limitinize takılmadan çalışın.** Claude, Codex ve Antigravity'nin 5 saatlik ve haftalık limitlerini birkaç dakikada bir güncel olarak görür, ne zaman dolacaklarını önceden öğrenirsiniz.

## Öne çıkan özellikler

- **Limitlerinizi kotanızdan harcamadan izleyin.** Limit yüzdeleri Claude Code'un, Codex'in ve Antigravity CLI'ın kendi resmi arayüzlerinden okunur. Bunun için modele tek bir istek bile gönderilmez.
- **Gereken aracı tek tıkla kurun.** Limit okumak için Claude Code, Codex CLI ya da Antigravity CLI gerekiyorsa uygulama onu üreticinin resmi kurulumuyla kurar. Giriş yapmanız gerekiyorsa ne yapacağınızı adım adım gösterir.
- **Limitinizin ne zaman dolabileceğini görün.** Tahmin, pencerenin başından bu yana ortalama hızınıza dayanır. Kısa süreli bir yoğunluk, bütün hafta öyle geçecekmiş gibi hesaplanmaz.
- **Limit pencerenizin tahmini API eşdeğerini görün.** Uygulama, sonuna kadar izlediği pencerelerden yola çıkarak 5 saatlik ya da haftalık bir pencerenin API fiyatlarıyla ne kadar kullanıma denk geldiğini hesaplar.
- **Size uygun planı öğrenin.** Limitleriniz sık doluyorsa bir üst plan önerilir. Daha küçük bir plan ise yalnızca sağlayıcının yayımladığı oranlar bunu destekliyorsa önerilir.
- **Aynı işin başka bir modelle ne tutacağını karşılaştırın.** İstekleriniz aynı token sayıları ve bağlam boyutlarıyla fiyat listesindeki her modele göre yeniden fiyatlanır.
- **Alt ajanların ve araçların payını görün.** Maliyetin ne kadarının alt ajanlardan geldiğini ve hangi araçların ne sıklıkla hata verdiğini görürsünüz.
- **Önbelleğin size ne kazandırdığını ölçün.** İsabet oranını, net tasarrufu ve uzun bir aradan sonra önbelleğin yeniden yazılmasının maliyetini görürsünüz.
- **Geçmişinizi kaybetmeyin.** Devam ettirdiğiniz ya da kopyasından yeni oturum açtığınız sohbetler iki kez sayılmaz. Claude Code kayıtlarını 30 gün sonra silse bile geçmişiniz uygulamanın veri tabanında kalır.

![Kısa bir tur: grafik ipucu ve abonelik değeriyle genel bakış, Antigravity dahil güncel limitler, limit geçmişi, öneriler, günlük görünüm, model karşılaştırması, canlı yakalama ve widget'ın yerleşimini değiştiren Widget stüdyosu](assets/readme/demo-tr.gif)

## Özellikler

### Limitlerinizi anlık izleyin

5 saatlik ve haftalık limit yüzdeleriniz Claude Code'un, Codex'in ve Antigravity CLI'ın kendi arayüzlerinden okunur; gördüğünüz sayılar araçların gösterdiğiyle aynıdır. Her pencerenin ne zaman dolacağını ve her projenin limitin ne kadarını kullandığını görürsünüz.

![Limitler sayfası: Claude, Codex ve Antigravity'nin 5 saatlik ve haftalık pencereleri, dolma tahminleri ve proje paylarıyla](assets/readme/limits-tr.webp)

Sağlayıcılar limitlerin sayısal karşılığını yayımlamıyor, yalnızca nasıl işlediklerini açıklıyor. Bu yüzden uygulama şu kurallara uyar:

- Gerçek yüzde yalnızca araç bunu kendi kaydına yazdığında ya da uygulama canlı okuduğunda gösterilir (bkz. [Canlı yakalama](#canlı-yakalama)).
- Son okumadan sonra aracı kullandıysanız güncel değer bilinemez. Bu durumda "?" görürsünüz; yanında son okuma ve ne kadar önce yapıldığı yazar. Eski bir değer hiçbir zaman güncelmiş gibi gösterilmez.
- Yüzdeleri **kullanılan** ("%15 kullanıldı", Claude'daki gibi) ya da **kalan** ("%85 kaldı", Codex'teki gibi) olarak görebilirsiniz. Bu tercihi **Ayarlar**'dan, **Limitler** sayfasından ya da **Widget stüdyosu**'ndan değiştirirsiniz. Uyarı renkleri her iki durumda da kullanıma göre değişir.
- Okuma yoksa her pencere için kendi bütçenizi belirleyebilirsiniz, örneğin "5 saatte 50 $ API eşdeğeri". Bu bütçeye göre hesaplanan yüzde **Tahmini** olarak gösterilir.
- Limitler hesabınızın tamamı için geçerlidir. Bir projenin payı, o penceredeki API eşdeğeri maliyetten aldığı paya göre hesaplanır ve **Tahmini** olarak gösterilir.

**Dolma tahmini.** Uygulama her güncel okumada, pencerenin **başından bu yana ortalama hızınıza** göre bir tahmin yapar (şu anki yüzde ÷ pencere başladığından bu yana geçen süre). Her pencere %0'dan başladığı için bu hız kesin olarak bilinir ve boşta geçen saatleri de içerir; kısa bir yoğunluk hiç bitmeyecekmiş gibi ileriye taşınmaz. Sonuçta pencerenin ne zaman dolacağını ("Paz 21:12 civarında dolacak, sıfırlanmadan 9 sa önce") ya da sıfırlandığında hangi seviyede olacağını görürsünüz. Pencerenin ilk onda birinde tahmin yapılmaz. Tahminler **Tahmini** etiketi taşır.

### Geçmiş limit pencerelerinizi inceleyin

**Limitler** sayfasındaki **Geçmiş** sekmesinde her 5 saatlik ve haftalık pencerenizi görürsünüz: ne kadar dolduğunu, dolup dolmadığını, ne kadar sürede dolduğunu ve o sırada bu bilgisayarda ne kadar kullandığınızı. Uygulama bu pencerelerden yola çıkarak bir pencerenin tamamının API eşdeğeri olarak kaça denk geldiğini de tahmin eder.

![Limit geçmişi: pencere istatistikleri, bir pencerenin API değeri ve her pencerenin en yüksek kullanımını gösteren grafik](assets/readme/history-tr.webp)

Her pencere için ne kadar süre dolu kaldığı ve sonuna kadar izlenip izlenmediği de yazar. Kullanım token ve API eşdeğeri olarak gösterilir. Sonu izlenmemiş pencereler taralı çizilir; bunların en yüksek okuması "en az" olarak okunmalıdır.

- **Filtreler:** sağlayıcı, pencere (5 saatlik ya da haftalık), dönem (7 günden tümüne kadar), durum (dolanlar, %80 ve üzeri, sonuna kadar izlenenler).
- **Özet:** kaç pencere olduğu, kaçının dolduğu ve toplamda ne kadar süre dolu kaldığı, en yüksek kullanımın ortalaması ve medyanı, ortalama dolma süresi ve en yüksek kullanımların dağılımı.
- **Bir pencerenin tamamı API cinsinden** (tahmini): Sonuna kadar izlenmiş ve en az %10 dolmuş her pencerede, bu bilgisayardaki API eşdeğeri kullanım en yüksek okumaya kadar toplanır ve o yüzdeye bölünür. Gösterilen değer bu sonuçların medyanı ve aralığıdır. Aynı limiti başka bir cihazda, web'de ya da masaüstü sohbette de kullandıysanız gerçek değer daha yüksektir.
- **Tablo:** tüm pencereler; başlangıca, en yüksek kullanıma, dolma süresine ya da yerel kullanıma göre sıralanabilir.

### Size uygun planı ve tasarruf fırsatlarını bulun

**Öneriler** sayfası kendi kayıtlarınızdan bulgular çıkarır. Her bulgu dayandığı sayıları, ne yapabileceğinizi ve nasıl hesaplandığını gösterir. **Plan önerisi** de limitleriniz sık doluyorsa bunu söyler; daha küçük bir planı ise yalnızca yayımlanmış oranlar destekliyorsa önerir.

![Öneriler sayfası: Claude ve Codex için plan önerileri, önbelleğin yeniden yazılması ve uzun bağlam ek ücreti bulguları](assets/readme/tips-tr.webp)

**Plan önerisi.** Son 28 güne bakar; yalnızca bu ölçümlere ve sağlayıcıların yayımladığı bilgilere dayanır:

- **Limitleriniz dolduysa** (bir haftalık pencere bir kez ya da 5 saatlik pencereler üç kez): Bir üst plan, fiyat farkı ve yayımlanmış oranıyla birlikte önerilir. Claude'da Max 5x ve Max 20x, Pro'nun oturum başına kullanım hakkının 5 ve 20 katını verir. ChatGPT'de Pro planının 5 saatlik limiti yoktur. Haftalık limitin planlar arasında nasıl arttığı yayımlanmadığı için kart bunu ayrıca belirtir.
- **Daha küçük plan:** Yalnızca oturum oranının yayımlandığı durumlarda (Claude) önerilir. Bunun için en az 14 günlük okuma ve sonuna kadar izlenmiş en az iki haftalık ve beş adet 5 saatlik pencere gerekir. Ayrıca her pencerenin alt planda %80'in altında kalması gerekir. Haftalık kısım kesin bir bilgi olarak değil, bir koşul olarak yazılır ("haftalık hak da aynı oranda olsaydı"). OpenAI planları arasındaki oranı yayımlamadığı için ChatGPT'de daha küçük plan önerilmez.
- **7 günden az okuma varsa** hiçbir öneri yapılmaz.

Claude okumaları iki Max planı için de yalnızca "max" der; bu yüzden planınızı **Limitler** sayfasından seçin. Seçtiğiniz plandan farklı bir planda okunmuş Claude pencereleri (Pro ile Max arasında) hesaba katılmaz. Codex okumaları planın adını tam olarak içerir ve siz plan seçmediyseniz bu bilgi kullanılır.

**Bulgular.** Her kuralın sabit bir eşiği vardır. Bir bulgunun gösterilmesi için tutarın en az 0,50 $ olması ve dönem maliyetinin en az %1'ine ulaşması gerekir. Uygulama şunlara bakar:

- Uzun bir aradan sonra önbelleğin baştan yazılması. Claude, önbelleğe aldığı içeriği bir süre kullanılmazsa (5 dakika ya da 1 saat) siler. Ara verip aynı sohbete döndüğünüzde bu içerik yeniden yazılır ve daha pahalıya gelir. Uygulama bu farkı, aynı içerik önbellekten okunsaydı ne tutacağıyla karşılaştırarak gösterir. Yalnızca daha önce önbellekte olan içerik hesaba katılır; sohbete yeni eklediğiniz kısım sayılmaz.
- Uzun bağlam, hızlı mod ve veri yerleşimi ek ücretleri. Aynı isteklerin standart fiyatıyla karşılaştırılır.
- Maliyetin büyük kısmının bağlamı 100K token'ı aşan isteklerden gelmesi.
- Çağrılarının %25'i ya da daha fazlası hata veren araçlar (sonucu bilinen en az 20 çağrı).

Tutarlar API eşdeğeridir. Abonelikle kullanıyorsanız bu tutarları ödemezsiniz; tutarlar kullanımınızın büyüklüğünü gösterir.

**Abonelik değeri.** **Genel bakış** sayfası, son 30 günde her sağlayıcıdaki API eşdeğeri kullanımınızı planınızın bu günlere düşen ücretiyle karşılaştırır ("4,7×"). Birikimli kullanımınız plan ücretiyle aynı grafikte çizilir ve planın kendini kaçıncı gün amorti ettiği gösterilir.

- **Liste fiyatları** [`config/plans.json`](config/plans.json) dosyasındadır ve 2026-10-02'de resmi sayfalardan doğrulanmıştır ([Claude](https://claude.com/pricing), [ChatGPT](https://learn.chatgpt.com/docs/pricing)).
- **Kendi fiyatınız:** Sabit fiyatı olmayan planlarda (ChatGPT Pro, Enterprise) fiyatı siz girersiniz. Her planın fiyatını **Limitler** sayfasından değiştirebilirsiniz (örneğin yıllık ödüyorsanız).
- **Alt sınır:** Claude ve ChatGPT'deki sohbet kullanımı yerel kayıtlarda olmadığı için gerçek değer daha yüksek olabilir.

### Kullanımınızı gün gün görün

**Günlük** sayfası seçtiğiniz dönemi gün gün gösterir. Kısa dönemlerde sütunlar, uzun dönemlerde takvim görürsünüz; değerleri token, API eşdeğeri ya da istek sayısı olarak seçebilirsiniz.

![Günlük sayfası: Claude Code, Codex ve Antigravity için günlük sütunlar, en yoğun gün ve aktif gün serileri](assets/readme/daily-tr.webp)

Sayfanın üstünde en yoğun gününüz, aktif gün başına ortalamanız, en uzun ve şu anki aktif gün seriniz ve hafta sonu payınız yer alır. Bir gün seçtiğinizde ya da aktif günler arasında gezindiğinizde o günün ayrıntıları açılır: token, maliyet, istek, oturum, ilk ve son etkinlik, önbellek isabeti, saat saat grafik, kullandığınız araçlar, modeller ve projeler, o gün ulaşılan en yüksek limit okumaları. Yanında haftanın her gününün ortalamasını ve en yoğun günlerinizi görürsünüz.

### Aynı işin başka bir modelle ne tutacağını görün

**Dağılım** sayfası, ilgili dönemdeki isteklerinizi aynı token sayıları ve bağlam boyutlarıyla fiyat listesindeki her modele göre yeniden hesaplar. Sonuçları gerçek maliyetinizle birlikte, en ucuzdan en pahalıya doğru görürsünüz.

![Gerçek maliyeti, aynı isteklerin diğer Claude, GPT ve Gemini modelleriyle tutacağı maliyetle karşılaştıran grafik](assets/readme/compare-tr.webp)

Model aileleri aynı metni farklı sayıda token'a böler (Claude 4.7 ve sonrası yaklaşık %30 daha fazla token üretir). Bu yüzden farklı aileler arasındaki karşılaştırma yaklaşıktır; kalite ve hız hesaba katılmaz.

### Alt ajanların ve araçların payını görün

**Dağılım** sayfası, alt ajanlarınızın ana konuşmanın yanında ne kadar tuttuğunu, ajanların hangi araçları çağırdığını ve bu çağrıların ne sıklıkla hata verdiğini gösterir.

![Alt ajanlar tablosu: istek, token ve maliyetin ana konuşma ile Explore gibi alt ajanlar arasındaki dağılımı](assets/readme/agents-tr.webp)

- **Alt ajanlar:** Alt ajanların (Explore gibi Claude Code ajanları, Codex'in guardian otomatik incelemesi vb.) istek, token ve API eşdeğeri maliyeti, ana konuşmayla yan yana gösterilir. Bir alt ajanın kullanımı onu başlatan oturuma sayılır.
- **Araç kullanımı:** Her aracın kaç kez çağrıldığını ve çağrıların ne kadarının hata verdiğini görürsünüz. Hata bilgisi, aracın kendi kaydettiği sonuçtan alınır. MCP araçları sunucuya göre gruplanır. Yalnızca aracın adı ve sonucu saklanır; girdileri ve çıktıları hiçbir zaman saklanmaz.

### Oturumlarınızı, önbelleği, bağlamı ve projelerinizi inceleyin

| ![Oturumlar sayfası](assets/readme/sessions-tr.webp)                              | ![Önbellek sayfası](assets/readme/cache-tr.webp)                                         |
| --------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------- |
| **Oturumlar**: her oturumunuz; süresi, modelleri, maliyeti ve en büyük bağlamıyla | **Önbellek**: isabet oranı, yeniden kullanım ve önbelleğin size gerçekte ne kazandırdığı |

| ![Bağlam sayfası](assets/readme/context-tr.webp)                      | ![Projeler sayfası](assets/readme/projects-tr.webp) |
| --------------------------------------------------------------------- | --------------------------------------------------- |
| **Bağlam**: istemlerinizin ne kadar büyüdüğü ve uzun bağlam ek ücreti | **Projeler**: proje ve git dalı bazında kullanım   |

**Oturumlar** sayfası dönemdeki her oturumunuzu listeler: başlangıç, proje, modeller, süre (ilk ve son istek arası), istek, token, API eşdeğeri, önbellek isabeti ve en büyük bağlam. Listeyi sıralayabilir ve içinde arama yapabilirsiniz; bir satırı açınca model bazında ayrıntısını görürsünüz. Alt ajan kayıtları ait oldukları oturuma sayılır.

**Bağlam** sayfası her istekte gönderilen bağlamın büyüklüğünü gösterir (önbellekten okunan ve önbelleğe yazılanlar dahil tüm girdi token'ları):

- ortalama, medyan, p90 ve en büyük değer,
- boyut dağılımı (10K altından 1M üstüne kadar),
- günlük ortalama ve p90,
- model bazında dağılım ve **uzun bağlam ek ücreti**: bir modelin uzun bağlam eşiğini (GPT-5.x için 272K) aşan isteklerin standart fiyata göre fazladan tuttuğu tutar.

**Önbellek** sayfasında şunları görürsünüz:

- isabet oranı (önbellekten okunan ÷ istem token'ları),
- yeniden kullanım katsayısı (okuma ÷ yazma). Claude için ve önbellek yazmalarını kaydeden Codex modelleri için hesaplanır; eski Codex kayıtları yalnızca okumaları tuttuğu için hesaba katılmaz,
- 5 dakikalık ve 1 saatlik yazmaların ayrımı,
- günlük isabet oranı,
- model, araç ve projeye göre dağılım,
- **net tasarruf**: önbellekten okunan token'ları tam fiyat yerine indirimli ödediğiniz için kazandığınız tutar; önbelleğe yazma ücreti bundan düşülür. Değer, API eşdeğeridir ve her istek kendi modelinin fiyatıyla hesaplanır.

Net tasarruf eksi çıkıyorsa, yazılan önbellek maliyetini karşılayacak kadar okunmamış demektir.

**Projeler** sayfası her projenin kullanımını git dalına göre ayırır. Claude Code dalı her istekte, Codex ise oturum bir depoda başladığında kaydeder. Depo dışındaki istekler "dal yok" olarak görünür.

### Limitlerinizi masaüstünde görün

Widget her zaman üstte duran küçük bir penceredir. Varsayılan olarak ekranın sağ alt köşesinde durur. Neleri gösterdiğini ve nasıl göründüğünü **Widget stüdyosu**'nda canlı önizlemeyle değiştirirsiniz.

![Bugünkü token ve maliyeti, Claude, Codex ve Antigravity'nin 5 saatlik limit halkalarıyla gösteren masaüstü widget'ı](assets/readme/widget-tr.webp)

![Widget stüdyosu: canlı önizleme ve widget'ın gösterebileceği öğelerin listesi](assets/readme/studio-tr.webp)

**Widget stüdyosu**'nda şunları ayarlayabilirsiniz:

- **Gösterilenler:** ana sayı, ikincil değer, 5 saatlik ve haftalık limitler, araçlara göre dağılım, 7 ve 30 günlük toplamlar, son güncelleme saati. Her öğeyi açıp kapatabilir ve sırasını değiştirebilirsiniz.
- **Yerleşim:** yatay, dikey ya da tek satır.
- **Limit gösterimi:** halka, çubuk ya da metin.
- **Görünüm:** tema, vurgu rengi, ölçek, arka plan opaklığı, köşe yuvarlaklığı, kenarlık, gölge, etiketler, sıfırlanmaya kalan süre.
- **Yazı:** bilgisayarınızda yüklü herhangi bir yazı tipi (aranabilir; her biri kendi görünümüyle listelenir), yazı boyutu, ana sayı boyutu, sayı kalınlığı, sabit genişlikli rakamlar.
- **Uyarı eşikleri:** dikkat ve kritik seviyeleri.
- **Davranış:** her zaman üstte, konum kilidi, tıklayınca ne olacağı, tam ekran uygulamalarda gizlenme (Windows), köşeye yerleştirme, hangi sağlayıcıların gösterileceği.

Widget boyutunu içeriğe göre kendisi ayarlar. Bir köşeye sabitlediyseniz büyüyüp küçülürken o köşede kalır; sürüklerseniz bıraktığınız yerde durur. Sağ tıklayarak hızlı ayarlara ulaşabilirsiniz. Sistem tepsisindeki simge her zaman görünür.

- **Tepsi simgesi** (Windows'ta saatin yanında, macOS'te menü çubuğunda, Linux'ta sistem tepsisinde): Bir limitin yüzdesini (o an en dolu olanı ya da sizin seçtiğinizi) widget'ın uyarı renkleriyle gösterebilir. Üzerine geldiğinizde tüm limitler listelenir. Ayarı **Ayarlar → Tepsi simgesi** bölümündedir.
- **Kısayol:** Sistem genelinde bir tuş kısayoluyla widget'ı gösterip gizleyebilirsiniz (varsayılan **Ctrl+Alt+Shift+W**, macOS'te **⌃⌥⇧W**). Değiştirmek için **Ayarlar → Widget** bölümünü kullanabilirsiniz. Başka bir programın kullandığı kısayol kabul edilmez.

### Haftalık rapor alın, verilerinizi yönetin

Haftalık kullanımınızı PDF olarak kaydedebilir, verilerinizi dışa aktarıp yedekleyebilir ve fiyatları kendinize göre düzenleyebilirsiniz.

**Haftalık özet (PDF).** **Ayarlar → Haftalık özet (PDF)** bölümünden geçen haftayı, bu haftayı ya da son 7 günü A4 PDF olarak kaydedebilirsiniz. Özette şunlar yer alır: önceki döneme göre değişimiyle toplamlar, günlük grafik, araçlar, abonelik değeri, tahminleriyle güncel limitler, bağlam, modeller, projeler (gizlediğiniz adlar gizli kalır), en maliyetli oturumlar ve notlar. Yazdırma penceresi açılmaz. İsterseniz son tam haftanın (pazartesi–pazar) özeti, klasörde henüz yoksa her hafta kendiliğinden kaydedilir. Bilgisayar kapalıyken kaçırılan hafta bir sonraki açılışta kaydedilir (varsayılan olarak Belgeler klasörünün içindeki **AI Usage Tracker** klasörüne kaydedilir).

**Kalıcı arşiv.** Araçlar kendi kayıtlarını zamanla silebilir. Örneğin Claude Code CLI eski oturumları varsayılan olarak **30 gün** sonra siler; Claude Desktop/Cowork oturumları bunun dışındadır. Uygulama içe aldığı her kaydı kendi veri tabanında tuttuğu için geçmişiniz kaybolmaz. Hiçbir kayıt iki kez sayılmaz (bkz. [Nasıl çalışır](#nasıl-çalışır)).

**Dışa aktarma ve yedek.** **Ayarlar → Veriler** bölümünde şunları yapabilirsiniz:

- CSV ya da JSON olarak dışa aktarma (istek bazında ya da günlük özet),
- veri tabanını yedekleme,
- bir yedeği mevcut verilerle birleştirerek içe aktarma,
- **Tüm verilerimi sil:** Veri tabanını boşaltır ve yeniden onaylayana kadar kayıt almayı durdurur. Başka bir yere kaydettiğiniz yedeklere ve dışa aktarımlara dokunmaz.

**Fiyatlar.**

- Fiyatlar [`config/pricing.json`](config/pricing.json) dosyasındadır ve 2026-10-02'de resmi sayfalardan doğrulanmıştır: [Anthropic](https://platform.claude.com/docs/en/about-claude/pricing), [OpenAI](https://developers.openai.com/api/docs/pricing).
- Hesaba katılan fiyat kalemleri: önbelleğe yazma (5 dk / 1 sa), önbellekten okuma, OpenAI uzun bağlam katmanı (>272K), hızlı mod (fast mode) ve ABD veri yerleşimi çarpanları, web araması ücreti.
- Fiyatı bilinmeyen modeller **maliyete eklenmez** ve "fiyat tanımsız" uyarısıyla listelenir. Ad benzerliğine bakılarak tahmini fiyat atanmaz. **Genel bakış**'taki bu uyarıyı kapatabilirsiniz; yalnızca başka bir model fiyatsız kalırsa yeniden çıkar. Uyarıyı **Ayarlar → Fiyatlar → Uyarıyı tekrar göster** ile de geri getirebilirsiniz.
- Fiyatları **Ayarlar → Fiyatlar** ekranında düzenleyebilirsiniz; düzenlemeleriniz uygulamanın veri klasöründeki `pricing.json` dosyasına kaydedilir (bkz. [Gizlilik](#gizlilik)). Aynı ekranda fiyatsız bir modeli başka bir modelin fiyatıyla hesaplatabilir ya da varsayılan fiyatlara dönebilirsiniz.

## Desteklenen kaynaklar

| Kaynak                                                 | Okunan konum                                                             | Token     | Limit                      | Not                                                                                 |
| ------------------------------------------------------ | ------------------------------------------------------------------------ | --------- | -------------------------- | ----------------------------------------------------------------------------------- |
| **Claude Code** (CLI, VS Code, Claude masaüstü "Code") | `$CLAUDE_CONFIG_DIR` ya da `~/.claude/projects/**/*.jsonl` | **Kesin** | Yalnızca limit dolduğunda  | Yardımcı arka plan çağrıları (örneğin web araması özetleme) bu kayıtlarda yer almaz |
| **Cowork oturumları** (Claude masaüstü)                | *Claude klasörü*`/local-agent-mode-sessions/**` | **Kesin** | **Kesin %** (5 sa / 7 gün) | Yalnızca bilgisayarınızda çalışan işler. Yeni Cowork işleri bulutta çalışır: token sayıları bilgisayarınızda tutulmaz ancak plan limitlerinize dahil edilir |
| **Claude masaüstü: plan kullanımı**                    | *Claude klasörü*`/plan-usage-history.json` | —         | **Kesin %**                | Sohbet token'ları yerelde tutulmadığı için token verisi **yok**                     |
| **Codex** (CLI ve masaüstü)                            | `$CODEX_HOME` ya da `~/.codex/{sessions,archived_sessions}` | **Kesin** | **Kesin %** (her istekte)  |                                                                                     |
| **Antigravity** (uygulama, IDE ve `agy` CLI)           | `~/.gemini/{antigravity,antigravity-ide,antigravity-cli}/conversations/*.db` ya da `$ANTIGRAVITY_APP_DATA_DIR` | **Kesin** | **Yakalanan %** (`agy` ile) | Veritabanının bir kopyası okunur; Antigravity'nin dosyalarına dokunulmaz. Gemini ile Claude/GPT modellerinin limitleri ayrı gösterilir |
| ChatGPT masaüstü                                       | —                                                                        | **Yok**   | —                          | Yalnızca algılanır                                                                  |

`~` kullanıcı klasörünüzü gösterir (Windows'ta `%USERPROFILE%`, örneğin `C:\Users\adınız`). *Claude klasörü* Windows'ta `%APPDATA%\Claude`, macOS'te `~/Library/Application Support/Claude` klasörüdür; Claude masaüstü uygulamasının Linux sürümü yoktur.

Her sayının yanında bir doğruluk etiketi bulunur:

- **Kesin:** Aracın kendi kaydındaki değer.
- **Tahmini:** Uygulamanın hesapladığı değer; örneğin bir projenin limit payı ya da belirlediğiniz bütçeye göre hesaplanan yüzde.
- **Yakalanan:** Uygulamanın canlı okuduğu değer (bkz. [Canlı yakalama](#canlı-yakalama)): limit okuma, durum satırı köprüsü ya da yerel telemetri alıcısı üzerinden gelir.

## Kurulum

[Son sürümden](../../releases/latest) sisteminize uygun dosyayı indirin. İlk açılışta bilgisayarınızda bulunan araçlar listelenir. İstediğiniz kaynakları seçip **Başla**'ya basın. Siz onay vermeden hiçbir kayıt okunmaz.

### Windows

Windows 10 ve 11.

1. `AI-Usage-Tracker_x.y.z_x64-setup.exe` dosyasını çalıştırın. Kurulum yalnızca sizin kullanıcı hesabınıza yapılır ve yönetici yetkisi istemez; uygulama `%LOCALAPPDATA%\AI Usage Tracker` klasörüne kurulur.
2. WebView2 Windows 11'de zaten yüklüdür. Windows 10'da eksikse kurulum dosyası onu da yükler.

> **SmartScreen uyarısı.** Kurulum dosyası henüz kod imzası taşımadığı için Windows "Bilinmeyen yayımcı" uyarısı gösterebilir. Bu durumda **Ek bilgi → Yine de çalıştır**'ı seçin.

**Kaldırma.** **Ayarlar → Uygulamalar → AI Usage Tracker → Kaldır** yolunu izleyin. Uygulama kaldırılırken **Canlı yakalama** ile Claude Code ayarlarında yapılan değişiklikler geri alınır. "Uygulama verilerini sil" kutusunu işaretlerseniz `%LOCALAPPDATA%\AIUsageTracker` klasörü de silinir. "Windows ile başlat" kaydı her durumda temizlenir.

### macOS

macOS 11 ve sonrası; Apple Silicon ve Intel Mac'lerde çalışır.

1. `AI-Usage-Tracker_x.y.z_universal.dmg` dosyasını açın ve **AI Usage Tracker**'ı **Uygulamalar** klasörüne sürükleyin.
2. Uygulamayı **Uygulamalar** klasöründen açın. Apple tarafından imzalanıp onaylandığı (notarize) için uyarı göstermeden açılır.

Uygulamayı disk görüntüsünden değil, Uygulamalar klasöründen çalıştırın. Disk görüntüsünden açılan kopya oturum açılışında başlayamaz, Claude Code durum satırını koruyamaz ve kendini güncelleyemez; böyle açtığınızda uygulama sizi uyarır.

**Kaldırma.** macOS'te uygulama silinirken bu değişiklikler geri alınamadığı için önce **Kaynaklar → Canlı yakalama** bölümünde açtığınız anahtarları ve **Ayarlar → Oturum açınca başlat** seçeneğini kapatın. Ardından uygulamayı Çöp Sepeti'ne sürükleyin. Verileriniz siz silene kadar `~/Library/Application Support/AIUsageTracker` klasöründe kalır.

### Linux

Debian 12, Ubuntu 22.04 ve sonrası, Fedora, openSUSE ve WebKitGTK 4.1 bulunan diğer x86_64 dağıtımlarda çalışır.

- **Debian, Ubuntu, Mint:** `AI-Usage-Tracker_x.y.z_x86_64.deb` dosyasını `sudo apt install ./AI-Usage-Tracker_x.y.z_x86_64.deb` ile kurun.
- **Fedora:** `AI-Usage-Tracker_x.y.z_x86_64.rpm` dosyasını `sudo dnf install ./AI-Usage-Tracker_x.y.z_x86_64.rpm` ile kurun.
- **openSUSE:** Paketler GPG imzası taşımadığı için zypper'a `sudo zypper install --allow-unsigned-rpm ./AI-Usage-Tracker_x.y.z_x86_64.rpm` komutunu verin. Önce dosyayı `SHA256SUMS` ile doğrulayabilirsiniz (bkz. [Dosyanın doğruluğunu kontrol edin](#dosyanın-doğruluğunu-kontrol-edin)).
- **Her dağıtım:** `AI-Usage-Tracker_x.y.z_x86_64.AppImage` dosyasını çalıştırılabilir yapın (`chmod +x`) ve açın. Hiçbir şey kurulmaz. Açılmazsa FUSE 3'ü kurun (Debian ve Ubuntu'da `sudo apt install fuse3`) ya da dosyayı `APPIMAGE_EXTRACT_AND_RUN=1 ./AI-Usage-Tracker_x.y.z_x86_64.AppImage` ile başlatın.

Uygulama üç biçimde de kendini günceller. `.deb` ve `.rpm` için sisteminiz parolanızı sorar ve güncellemeyi paket yöneticiniz kurar. Bu mümkün olmazsa uygulama yeni paketi elle kurmanızı ister.

**GNOME'da tepsi simgesi.** GNOME tepsi simgelerini yalnızca AppIndicator eklentisiyle gösterir. Ubuntu'da hazır gelir; diğer GNOME sistemlerinde "AppIndicator and KStatusNotifierItem Support" eklentisini kurun.

**Widget kısayolu.** KDE Plasma ve GNOME'da, Wayland'de de, hangi programda olursanız olun çalışır. GNOME 47 ve öncesinde uygulama bunun için bir GNOME özel kısayolu ekler (Ayarlar → Klavye → Özel Kısayollar). Sway ya da Hyprland kullanıyorsanız `ai-usage-tracker --toggle-widget` komutuna kendiniz bir tuş atayın; örneğin Sway'de `bindsym Ctrl+Alt+Shift+w exec ai-usage-tracker --toggle-widget`, Hyprland'de `bind = CTRL ALT SHIFT, W, exec, ai-usage-tracker --toggle-widget`.

**Kaldırma.** `sudo apt remove ai-usage-tracker` ya da `sudo dnf remove ai-usage-tracker`. Paketi kaldırdığınızda, uygulamayı kullanan her kullanıcı için **Canlı yakalama** ile Claude Code ayarlarında yapılan değişiklikler ve oturum açılışında başlatma kaydı geri alınır. AppImage kullanıyorsanız önce bu anahtarları kapatın, sonra dosyayı silin. Verileriniz siz silene kadar `~/.local/share/AIUsageTracker` klasöründe kalır. GNOME'da widget kısayolunu da **Ayarlar → Widget kısayolu → Kaldır** bölümünden ya da GNOME'un klavye ayarlarından kaldırın.

### Dosyanın doğruluğunu kontrol edin

Her sürümde, her dosyanın sağlama değerini içeren bir `SHA256SUMS` dosyası bulunur. Bu dosyayı indirdiğiniz dosyanın yanına koyup şu komutu çalıştırın:

- **Linux:** `sha256sum -c SHA256SUMS --ignore-missing`
- **macOS:** `shasum -a 256 -c SHA256SUMS --ignore-missing`
- **Windows (PowerShell):** `Get-FileHash .\AI-Usage-Tracker_x.y.z_x64-setup.exe` komutunun verdiği değeri `SHA256SUMS` içindeki ilgili satırla karşılaştırın.

## Gizlilik

> **Prompt ve yanıtlarınız arşive alınmaz.** Uygulama kayıtlardan yalnızca token sayısı, model ve zaman gibi bilgileri alır; prompt ve yanıt metinleri uygulamanın arşivine kaydedilmez. Kullanım verileriniz bilgisayarınızdan hiçbir yere gönderilmez ve uygulamada telemetri yoktur.

- **Okunanlar:** Araçların kendi kayıtlarındaki zaman damgası, model adı, proje klasörü, oturum kimliği, token sayıları ve limit yüzdeleri.
- **Saklananlar:** Yalnızca bu sayılar ve meta veriler. Hepsi uygulamanın veri klasöründeki `tracker.db` (SQLite) dosyasında durur: Windows'ta `%LOCALAPPDATA%\AIUsageTracker`, macOS'te `~/Library/Application Support/AIUsageTracker`, Linux'ta `~/.local/share/AIUsageTracker`. Proje adları bu bilgisayardan çıkmaz. İsterseniz adları tek tek ya da toplu olarak gizleyebilirsiniz; gizlenen adlar dışa aktarımlarda da maskelenir.
- **Uygulamanın arşivinde hiçbir zaman tutulmayanlar:** Prompt ve yanıtlar, dosya içerikleri, kimlik bilgileri ve erişim token'ları. Uygulama, araçların oturum anahtarlarını tutan dosyaları (`.credentials.json`, `auth.json`) **okumaz**.
- **Antigravity'nin geçici kopyası:** Antigravity, sohbetlerini metinleriyle birlikte bir veri tabanında tutar. Uygulama bu dosyaya dokunmamak için onu, yalnızca sizin erişebildiğiniz geçici bir klasöre kopyalar, sayıları okur ve kopyayı hemen siler. Uygulama zorla kapatılırsa kalan kopyalar, Antigravity bir sonraki okunduğunda 10 dakikadan eskiyse silinir. Sohbet metni uygulamanın arşivine hiçbir zaman alınmaz.
- **İnternet bağlantısı:** Uygulamanın kendisi internete yalnızca güncelleme denetimi için bağlanır. Bu denetimde yalnızca sürüm dosyası indirilir, hiçbir veri gönderilmez (**Ayarlar → Güncellemeler**). Varsayılan olarak açık gelen limit okumaları bilgisayarınızdaki Claude Code, Codex ve Antigravity CLI'ı çalıştırır; bu araçlar limit yüzdenizi kendi servislerinden sorar. Uygulama onları telemetri ve hata raporlaması kapalı olarak başlatır. Limit okumalarını **Kaynaklar → Canlı yakalama** bölümünden kapatabilirsiniz. Bir aracı uygulamadan kurarsanız kurulum dosyası üreticinin sitesinden indirilir. Fiyat ve plan dosyaları uygulamayla birlikte gelir. Döviz kuru için de internetten veri çekilmez; kuru kendiniz girersiniz.

### Canlı yakalama

**Kaynaklar → Canlı yakalama** bölümünde beş yöntem bulunur. **Claude, Codex ve Antigravity limit okuma varsayılan olarak açıktır.** Bu üçü için uygulama hiçbir dosyayı değiştirmez, modele istek göndermez ve yalnızca kendi Claude Code, Codex ya da Antigravity CLI oturumunuzu kullanır. Claude Code'un ayar dosyasını değiştiren diğer iki yöntem **varsayılan olarak kapalıdır**. Her yöntemin neyi değiştirdiği ekranda yazar; yöntemi kapattığınızda değişiklik geri alınır. Windows'ta uygulama, Linux'ta paket kaldırılırken bu değişiklikler kendiliğinden geri alınır; güncellemelerde ise dokunulmaz. macOS'te ve AppImage'da uygulamayı silmeden önce bu anahtarları kapatın.

![Canlı yakalama paneli: beş anahtar ve her birinin neyi değiştirdiği](assets/readme/capture-tr.webp)

| Yöntem                               | Ne yapar                                                                                                                                                                                                                                                                                                                                                                                                                                                                        | Neyi değiştirir                                                                                                                                                                                                        |
| ------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Claude limit okuma**               | Claude Code'un kendi kullanım sorgusuyla 5 saatlik ve haftalık limit yüzdenizi okur. Modele istek gönderilmediği için kotanızdan bir şey harcanmaz. Claude'u kullanırken 5 dakikada bir, kullanmadığınızda 15 dakikada bir güncellenir (Claude'un kullanım servisi daha sık sorulmasına izin vermez). Böylece Claude masaüstü sohbetleri, Code sekmesi, Cowork ve web kullanımı da yansır. Claude Code'da Pro/Max hesabıyla oturum açmış olmanız gerekir (`claude` → `/login`). | Hiçbir şeyi değiştirmez. Kendi Claude Code kurulumunuzu ve oturumunuzu kullanır. Okuma sırasında MCP sunucuları, hook'lar, telemetri, hata raporlaması ve otomatik güncelleme kapalıdır. Giriş bilgilerinize dokunmaz. |
| **Codex limit okuma**                | Codex'in resmi arayüzüyle 5 saatlik ve haftalık limit yüzdenizi 5 dakikada bir okur. Böylece Codex'i kullanmadığınız zamanlarda da değer güncel kalır.                                                                                                                                                                                                                                                                                                                          | Hiçbir dosyayı değiştirmez. Kendi Codex kurulumunuzu ve oturumunuzu analitik kapalı olarak kullanır. Giriş bilgilerinize dokunmaz.                                                                                     |
| **Antigravity limit okuma**          | Antigravity CLI'ın (`agy`) kendi `/usage` komutuyla Gemini ve Claude/GPT havuzlarının 5 saatlik ve haftalık limit yüzdenizi 5 dakikada bir okur. Antigravity uygulaması ve IDE bu okumayı yapamaz; `agy` gerekir. | Hiçbir dosyayı değiştirmez. Kendi `agy` kurulumunuzu ve Google oturumunuzu kullanır; okuma sırasında `agy` kendini güncellemez. Giriş bilgilerinize dokunmaz. |
| **Claude Code durum satırı köprüsü** | Claude Code'un resmi `statusLine` verisinden (Pro/Max) 5 saatlik ve haftalık limit yüzdenizi kaydeder. Zaten bir durum satırınız varsa aynı girdiyle çalışmaya devam eder. Yalnızca Claude Code'un durum satırı gösterdiği yerde (terminalde) çalışır; Claude masaüstündeki Code sekmesi durum satırı çalıştırmaz.                                                                                                                                                              | `~/.claude/settings.json` dosyasındaki `statusLine` alanı (önce yedeği alınır).                                                                                                                                        |
| **Yerel telemetri alıcısı**          | Claude Code'un resmi telemetri verisinden istek bazında token sayılarını alır. Kayıt dosyalarında görünmeyen yardımcı model çağrılarını da yakalar.                                                                                                                                                                                                                                                                                                                             | `~/.claude/settings.json` dosyasındaki `env` bloğuna yalnızca log dışa aktarma değişkenleri eklenir. Alıcı yalnızca `127.0.0.1` adresini dinler ve yalnızca bu kuruluma özel anahtarı taşıyan istekleri kabul eder.    |

Limit okuma için gereken araç kurulu değilse kartında **Kur** ve **Kurulum sayfası** düğmeleri çıkar. **Kur**, aracı üreticinin resmi kurulum betiğiyle kurar: Claude Code için `claude.ai/install`, Codex için `chatgpt.com/codex/install`, Antigravity CLI için `antigravity.google/cli/install`. Araç kurulu ama giriş yapılmamışsa genel bakışta ve ilgili kartta nasıl giriş yapacağınız adım adım yazar; **Tekrar dene** ile limitleri hemen okutabilirsiniz.

Güvenlik önlemleri:

- Hem telemetride hem kayıt dosyasında görünen bir istek bir kez sayılır.
- E-posta adresi ve hesap kimliği gibi bilgiler saklanmaz; prompt kaydı hiçbir zaman açılmaz.
- Kendi telemetri kurulumunuz varsa uygulama ona dokunmaz ve sizi uyarır.
- Bir anahtarı kapattığınızda yalnızca uygulamanın yazdığı değerler geri alınır; sizin yaptığınız değişiklikler korunur.

## Nasıl çalışır

Her şey sizin bilgisayarınızda olur:

1. Claude Code, Cowork, Claude masaüstü, Codex (CLI ve masaüstü) ve Antigravity (uygulama, IDE ve CLI) kullanım kayıtlarını diske yazar.
2. Uygulama bu kayıtları okur ve içlerinden yalnızca token sayısı, model ve zaman gibi bilgileri alır. Prompt ve yanıt metinleri uygulamanın arşivine kaydedilmez.
3. Limit okumaları açıksa limit yüzdeleri bilgisayarınızdaki `claude`, `codex` ve `agy` üzerinden alınır.
4. Yerel veri tabanına (SQLite) yalnızca sayılar ve meta veriler yazılır.
5. Panel, widget, tepsi simgesi ve PDF özet bu veri tabanını kullanır.

Kayıtlar her seferinde baştan değil, kalınan yerden okunur; yeniden tarama yalnızca yeni eklenen kısmı işler. Aynı yanıtın ara kayıtları ile devam ettirilen ya da kopyalanarak yeni oturuma dönüştürülen sohbetlerdeki tekrarlar ve arşive taşınan Codex dosyaları yalnızca bir kez sayılır.

## Güncellemeler

Uygulama açılışta ve 6 saatte bir yeni sürüm olup olmadığına bakar. Bunun için bu deponun sürümlerinden yalnızca küçük bir sürüm dosyası indirir ve hiçbir veri göndermez. Güncelleme yalnızca siz onayladıktan sonra ve imzası uygulamaya gömülü anahtarla eşleşirse kurulur. Denetimi **Ayarlar → Güncellemeler** bölümünden kapatabilirsiniz.

## Bilinen sınırlamalar

- **Claude Code toplamları:** Claude Code kayıtları ana model dışındaki yardımcı çağrıları içermez. Bu yüzden toplamlar gerçek kullanımın biraz altında kalabilir.
- **Masaüstü sohbetleri:** Claude masaüstü ve ChatGPT masaüstü sohbetlerinin token sayıları yerelde bulunmaz.
- **Codex hızlı katmanı:** Codex fast/priority katmanını istek bazında kaydetmediği için standart fiyat uygulanır.
- **Yalnızca telemetriden bilinen istekler:** Bu isteklerde önbellek yazmaları 5 dakikalık fiyattan hesaplanır, bu yüzden maliyetleri biraz düşük çıkabilir.
- **`codex-auto-review`:** Bu modelin resmi fiyatı yayımlanmamıştır.
- **Eski Antigravity konuşmaları:** Antigravity'nin eski sürümlerinin şifreli olarak sakladığı `.pb` konuşmaları ve veritabanı silinmiş konuşmalar okunmaz; bunlarda gerçek token sayısı bulunmaz.
- **Google AI plan fiyatları:** Ülkeye ve kampanyaya göre değiştiği için Antigravity planınızın aylık ücretini kendiniz girersiniz.
- **Kayıt biçimleri:** Araçların kayıt biçimleri resmi olarak belgelenmiş değildir ve sürümden sürüme değişebilir. Uygulama bilmediği alanları atlar ve tanıyamadığı kayıtları **Kaynaklar** ekranında listeler.

## Kaynaktan derleme

Gereksinimler: Rust 1.90+ ve Node 20.19+ veya 22.12+; bunlara ek olarak:

- **Windows:** Visual Studio Build Tools (C++)
- **macOS:** Xcode Command Line Tools
- **Linux:** WebKitGTK 4.1 ve geliştirme dosyaları; örneğin Debian ve Ubuntu'da `sudo apt install libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev libssl-dev`

```bash
npm --prefix ui install
cargo test -p tracker-core              # parser, hesaplama ve içe aktarma testleri
npx --prefix ui tauri dev               # geliştirme modu
npm --prefix ui run release             # paketler → target/release/bundle/release/
```

Yalnızca `npm --prefix ui run dev` komutunu çalıştırırsanız panel tarayıcıda örnek verilerle açılır.

| Klasör                | İçerik                                                                                                                 |
| --------------------- | ---------------------------------------------------------------------------------------------------------------------- |
| `crates/tracker-core` | Rust çekirdeği: keşif, ayrıştırıcılar, SQLite arşivi, fiyatlandırma, analiz, dışa aktarma (ve sentetik örnek kayıtlar) |
| `src-tauri`           | Masaüstü kabuğu: arka plan işçisi, dosya izleme, tepsi simgesi, widget, IPC                                            |
| `ui`                  | Svelte 5 arayüzü (panel ve widget)                                                                                     |
| `config`              | `pricing.json`, `plans.json`, `updates.json` (yayın yeri)                                                              |

Hata bildirimlerinizi, pull request'lerinizi ve geri bildirimlerinizi almaktan mutluluk duyarım.

## Kod imzalama politikası

Bkz. [CODE_SIGNING.md](CODE_SIGNING.md).

## Lisans

[MIT](LICENSE)
