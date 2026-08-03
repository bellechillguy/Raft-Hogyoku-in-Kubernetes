# Hogyoku Raft di Kubernetes

Repositori ini menjalankan key-value store berbasis Raft di Kubernetes. Node Raft berjalan sebagai StatefulSet, sedangkan UI bawaan berjalan sebagai Deployment terpisah.

File `api_gateway.rs` dan `index.html` tidak diubah. Perubahan pada aplikasi hanya mencakup node Raft, resolusi alamat, penyimpanan persisten, dan health endpoint.

## Cakupan implementasi

| Bagian | Implementasi |
| --- | --- |
| StatefulSet dan headless Service | Cluster dimulai dengan tiga pod, yaitu `raft-0` sampai `raft-2`. Setiap pod memiliki alamat DNS tetap melalui `raft-svc`. |
| API Gateway | API Gateway berjalan sebagai Deployment dengan dua replika dan diakses melalui ClusterIP Service. Nilai `RAFT_ADDR` dibaca dari ConfigMap. |
| ConfigMap | Konfigurasi ID node, daftar peer, port, lokasi data, ukuran awal cluster, dan contact node berada di `k8s/base/10-configmap.yaml`. |
| Penyimpanan persisten | Setiap pod memperoleh satu PVC `ReadWriteOnce` melalui `volumeClaimTemplates`. |
| Pemulihan setelah crash | Node menyimpan term, vote, log, commit index, membership, dan snapshot di `/data`. |
| Probe | Endpoint `/ready` baru berhasil setelah node mengetahui leader. Endpoint `/live` memeriksa apakah actor Raft masih merespons. |
| Scaling bonus | Pod dengan ordinal 3 dan 4 otomatis mengirim `AddNode` ke cluster yang sedang berjalan. |
| NetworkPolicy bonus | Semua trafik ditolak secara default. Port RPC hanya dapat diakses oleh pod Raft dan API Gateway, sedangkan trafik dari luar cluster hanya masuk melalui port HTTP gateway. |
| Namespace dan RBAC bonus | Semua resource berada di namespace `hogyoku`. ServiceAccount tidak menerima token maupun izin untuk mengakses Kubernetes API. |

## Tautan

- [Repository](https://github.com/bellechillguy/Raft-Hogyoku-in-Kubernetes)
- [Video demo](https://youtu.be/aY3Tl-msO1A)

## Mengapa memakai StatefulSet

Setiap node Raft memiliki identitas dan penyimpanan yang tidak boleh tertukar. Sebagai contoh, `raft-1` harus kembali sebagai node 2 dan menggunakan PVC yang sama setelah restart.

Deployment tidak memberikan hubungan tetap antara nama pod, hostname, dan volume. StatefulSet menyediakan ketiganya.

StatefulSet menggunakan konfigurasi berikut:

```yaml
podManagementPolicy: Parallel
```

Tanpa mode paralel, Kubernetes membuat pod secara berurutan. Kubernetes akan menunggu `raft-0` berstatus Ready sebelum membuat `raft-1`.

Masalahnya, `raft-0` belum dapat menjadi Ready karena cluster tiga node belum memiliki quorum. `podManagementPolicy: Parallel` membuat ketiga pod secara bersamaan sehingga proses election dapat dimulai tanpa menghilangkan identitas tetap milik StatefulSet.

Headless Service memberikan record DNS langsung kepada setiap pod. Node Raft tidak perlu menyimpan alamat IP yang dapat berubah setelah restart.

```text
raft-0.raft-svc.hogyoku.svc.cluster.local:8000
raft-1.raft-svc.hogyoku.svc.cluster.local:8000
raft-2.raft-svc.hogyoku.svc.cluster.local:8000
```

## Struktur direktori

```text
.
├── Dockerfile
├── docker-entrypoint.sh
├── k8s/
│   ├── base/
│   │   ├── 00-namespace-rbac.yaml
│   │   ├── 10-configmap.yaml
│   │   ├── 20-services.yaml
│   │   ├── 30-raft-statefulset.yaml
│   │   ├── 40-api-gateway.yaml
│   │   └── 50-network-policy.yaml
├── scripts/
│   ├── cluster-status.sh
│   └── measure-leader-failover.sh
├── src/
└── tests/
```

## Build dan deployment lokal

Siapkan Docker, `kubectl`, dan cluster Kubernetes yang aktif.

NetworkPolicy hanya dapat diuji jika CNI pada cluster mendukung dan menerapkan NetworkPolicy.

### Build image

Jalankan perintah berikut dari root repositori:

```bash
docker build -t hogyoku:local .
```

Docker Desktop Kubernetes dapat langsung memakai image lokal tersebut.

Untuk kind atau minikube, muat image ke dalam cluster terlebih dahulu:

```bash
kind load docker-image hogyoku:local
# atau
minikube image load hogyoku:local
```

### Deploy dari awal

```bash
kubectl delete namespace hogyoku --ignore-not-found
kubectl apply -k k8s
kubectl -n hogyoku get pods -w
```

Saat cluster baru menyala, kolom `READY` akan tetap menunjukkan `0/1` selama proses election berlangsung. Setelah salah satu node terpilih sebagai leader, ketiga pod Raft akan berubah menjadi `1/1`.

Periksa resource dan PVC dengan perintah berikut:

```bash
kubectl -n hogyoku get statefulset,deployment,service,pod,pvc
kubectl -n hogyoku get endpointslice \
  -l kubernetes.io/service-name=raft-svc
```

Untuk melihat status setiap node, jalankan:

```bash
./scripts/cluster-status.sh
```

Endpoint `/status` menampilkan role, term, leader ID, commit index, dan jumlah peer. Dalam kondisi normal, hanya satu node yang memiliki nilai berikut:

```json
"role": "leader"
```

## Membuka UI dan menguji data

Jalankan port-forward di terminal terpisah:

```bash
kubectl -n hogyoku port-forward service/api-gateway 8080:8080
```

Buka alamat berikut melalui browser:

```text
http://127.0.0.1:8080
```

Gunakan tombol **Set** dan **Get** pada UI untuk menyimpan dan membaca data.

Operasi yang sama dapat diuji langsung melalui HTTP:

```bash
curl -sS -X POST http://127.0.0.1:8080/api/set \
  -H 'content-type: application/json' \
  -d '{"key":"fragment","value":"still-here"}'

curl -sS http://127.0.0.1:8080/api/get/fragment
```

## Demo pemulihan setelah crash

Simpan data melalui UI, lalu cari node leader:

```bash
./scripts/cluster-status.sh
```

Hapus pod leader tanpa menghapus PVC:

```bash
kubectl -n hogyoku delete pod raft-LEADER_ORDINAL
kubectl -n hogyoku get pods -w
./scripts/cluster-status.sh
```

StatefulSet akan membuat pod pengganti dengan nama yang sama dan memasangkan kembali PVC miliknya. Dua node yang masih aktif akan memilih leader baru.

Setelah pod pengganti kembali Ready, baca lagi nilai `fragment` melalui UI. Nilai tersebut seharusnya masih tersedia.

### Mengukur waktu pergantian leader

Catat waktu sebelum pod dihapus. Hentikan pengukuran setelah status cluster kembali menunjukkan satu leader.

```bash
python3 -c 'import time; print(int(time.time() * 1000))'

kubectl -n hogyoku delete pod raft-LEADER_ORDINAL

while ! ./scripts/cluster-status.sh | grep -q '"role":"leader"'; do
  sleep 1
done

python3 -c 'import time; print(int(time.time() * 1000))'
```

Kurangi waktu akhir dengan waktu awal untuk memperoleh durasi recovery dalam milidetik.

Target pergantian leader pada tugas ini adalah kurang dari 10 detik.

## Scale dari tiga menjadi lima node

Jumlah replika dapat ditambah tanpa mengganti image atau me-restart tiga node lama.

```bash
kubectl -n hogyoku scale statefulset raft --replicas=5
kubectl -n hogyoku rollout status statefulset raft --timeout=180s
kubectl -n hogyoku get pods
./scripts/cluster-status.sh
```

`raft-3` menggunakan ID 4, sedangkan `raft-4` menggunakan ID 5. Kedua ID tersebut dibaca dari ConfigMap.

Entrypoint mendeteksi bahwa ordinal 3 dan 4 berada di luar ukuran awal cluster. Pod baru kemudian menghubungi `raft-0` untuk bergabung.

Jika `raft-0` bukan leader, RPC mengembalikan alamat leader. Proses join melanjutkan permintaan ke alamat tersebut. Leader lalu menulis perintah `AddNode` ke log dan mengirim log atau snapshot yang dibutuhkan node baru.

Setelah scaling selesai, ulangi operasi Get terhadap data yang dibuat sebelumnya. Nilainya harus tetap tersedia.

## Demo NetworkPolicy

Pastikan CNI cluster menerapkan NetworkPolicy.

Port HTTP API Gateway tetap dapat diakses, sedangkan port RPC Raft dari namespace lain harus ditolak.

```bash
kubectl run outside-check \
  --image=busybox:1.36 \
  --restart=Never \
  --rm -it \
  -- nc -zvw3 raft-0.raft-svc.hogyoku.svc.cluster.local 8000
```

Perintah tersebut seharusnya berakhir dengan timeout.

UI tetap dapat digunakan karena pod API Gateway mendapat izin khusus untuk mengakses port `8000` milik node Raft.

Periksa policy yang aktif dengan perintah berikut:

```bash
kubectl -n hogyoku get networkpolicy
kubectl -n hogyoku describe networkpolicy raft-internal
```

## Demo RBAC

Workload tidak perlu membaca Pod, Secret, atau resource Kubernetes lainnya. Karena itu, Role menggunakan konfigurasi berikut:

```yaml
rules: []
```

Token ServiceAccount juga tidak di-mount ke dalam pod. ServiceAccount milik Raft dan API Gateway hanya diikat ke Role kosong tersebut.

Uji hak akses keduanya:

```bash
kubectl auth can-i \
  --as=system:serviceaccount:hogyoku:raft \
  get pods -n hogyoku

kubectl auth can-i \
  --as=system:serviceaccount:hogyoku:api-gateway \
  get secrets -n hogyoku
```

Kedua perintah harus menghasilkan:

```text
no
```

Konfigurasi ini mengikuti prinsip least privilege karena workload tidak menerima izin yang tidak dipakai.

## Verifikasi source code

Jalankan pemeriksaan format, pengujian Rust, dan validasi manifest Kubernetes:

```bash
rustfmt --edition 2021 --check \
  src/raft/state.rs \
  src/raft/actor.rs \
  src/bin/server.rs \
  src/utils/client.rs \
  tests/persistent_recovery.rs

cargo test --locked --all-targets

kubectl kustomize k8s \
  >/tmp/hogyoku-rendered.yaml

kubectl kustomize k8s/overlays/https \
  >/tmp/hogyoku-https-rendered.yaml
```

Pengujian mencakup:

- operasi key-value;
- konflik log;
- snapshot;
- perintah perubahan membership; dan
- pemulihan data committed sebelum snapshot dibuat.
