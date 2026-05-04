# PocketHLE

> Высокоуровневый эмулятор (HLE) игр для Windows Mobile / Pocket PC.
> Архитектура и стиль кода вдохновлены проектами
> [touchHLE](https://github.com/touchHLE/touchHLE) (iPhone OS) и
> [EKA2L1](https://github.com/EKA2L1/EKA2L1) (Symbian).

PocketHLE не пытается эмулировать целое ядро Windows CE. Вместо этого, как
и `touchHLE`, эмулятор загружает реальный игровой `.exe`, запускает ARM-код
в эмуляторе процессора и реализует системные DLL (`coredll`, `aygshell`,
`gx`, `hss`...) на стороне хоста. Игра «думает», что она работает на
реальном Pocket PC.

Первая целевая ROM — небольшая физическая игра **JumpyBall** (ARM PE32,
Windows CE 5 GUI). Реализованные API соответствуют тому, что вызывает
именно эта игра.

> **Статус:** ранняя стадия. ARM-эмулятор работает, загрузчик читает PE,
> вызовы системных DLL перехватываются и трассируются, реализованы простые
> функции `coredll` (`memcpy`, `memset`, `GetTickCount` и т.п.). Графика
> ещё не выводится.

Английская версия → [`README.md`](README.md).

## Сборка на Linux

```bash
sudo apt install -y cmake build-essential pkg-config libclang-dev
rustup default stable      # 1.85+

# Базовая сборка (без настоящего CPU-бэкенда — быстро):
cargo build --release --workspace

# Полноценный билд с Unicorn Engine (примерно 3 минуты в первый раз):
cargo build --release -p pocket-cli --features unicorn

cargo test --workspace
```

## Запуск JumpyBall

```bash
# Просмотр содержимого CAB:
pockethle inspect-cab ~/JumpyBallPPC.cab

# Или распаковка вручную и запуск через Unicorn:
pockethle unpack-cab ~/JumpyBallPPC.cab /tmp/jumpy
pockethle -v run /tmp/jumpy/JUMPYB~1.002 \
    --cpu unicorn --max-slices 200 --instructions-per-slice 100000
```

В выводе вы увидите строчки вида
`unimplemented call -> COREDLL.dll!Rectangle` — это API, которые ещё нужно
реализовать в `crates/pocket-winceapi/src/coredll.rs`. Каждый «недостающий»
API — это маленький pull request на пару десятков строк.

## Дальнейшие планы

1. CRT-пролог: `__chkstk`, `_setjmp`, `longjmp`, `_except_handler3`.
2. Создание окна: `RegisterClassW`, `CreateWindowExW`, `SHFullScreen`.
3. Загрузка ресурсов: `FindResourceW`, `LoadResource`, `CreateFileW`,
   `ReadFile`.
4. GDI: софтверный растеризатор (`BitBlt`, `Rectangle`, `FillRect`).
5. GAPI: вывод фреймбуфера в окно SDL2 (Linux) и `SurfaceView` (Android).
6. Звук: реальное воспроизведение через SDL2 / OpenSL ES.
7. Ввод: клавиатура и тач → `WM_KEYDOWN` / `WM_LBUTTONDOWN`.

## Лицензия

Двойная лицензия: [Apache-2.0](LICENSE-APACHE) **ИЛИ** [MIT](LICENSE-MIT).
