# 미래 백로그 (Phase 2~4 · 선택 확장)

> 작성일: 2026-07-03
> 상태: **v1 범위 밖.** 자리(프로토콜 타입·메뉴 IA·데이터 정책)만 예약해 두고, 각 페이즈는 **앞 페이즈 완성 후** 착수 여부를 판단한다(선착수 금지).
> 근거: [06-포트폴리오-전략](06-포트폴리오-전략.md) §3, [07 ADR-005/006](07-결정기록-ADR.md). 로드맵 개요: [00 §14](00-개요-및-게임설계.md).

이 문서의 목적: 코어 스펙(00~04)을 **집중되게** 유지하기 위해, 미래 기능의 상세를 여기 모아 둔다. 프로토콜은 이 기능들을 **나중에 붙여도 v1이 깨지지 않도록**(미지 `t` 무시, 예약 타입) 준비돼 있다.

---

## Phase 2 — 파츠 소유·착용 영속화

- **영속성**: SQLite(임베디드, 파일 하나). 테이블: `parts`(카탈로그·가격), `users`, `user_parts`(소유), `user_loadout`(착용).
- **식별 = 패스워드리스 매직코드**: 이메일 6자리 코드 입력(링크 아님 → 폰/크로스디바이스 호스트 문제 없음).
  - 흐름: 이메일 → 코드 생성/저장(만료·1회용) → 코드 발송(개발=콘솔, 배포=이메일 API) → 코드 입력 검증 → 세션 발급 → WS 연결에 세션 첨부.
  - **보안 필수**: 시도횟수 제한(5회) + 짧은 만료(~10분) + 1회용 + 발송 레이트리밋.
  - `PUBLIC_URL` 설정값으로 로컬/LAN/터널/도메인 대응.
- 인증 요구 범위: **슬롯 참가·파츠 소유/착용에만**, **관전은 계속 익명**.
- 프로토콜 예약 타입: `auth`(요청/검증).
- 스키마 초안:
  ```sql
  users(user_id TEXT PK, email TEXT UNIQUE, verified_at INTEGER, balance INTEGER DEFAULT 0)
  login_tokens(code TEXT, user_id TEXT, expires_at INTEGER, attempts INTEGER)
  user_parts(user_id, part_id, acquired_at, PRIMARY KEY(user_id, part_id))
  user_loadout(user_id, slot, part_id, PRIMARY KEY(user_id, slot))
  ```

## Phase 3 — 샵 (인게임 재화 구매)

- **기능**: 카탈로그 열람(가격·잠금 표시) · 재화 잔액 · **구매**(서버 잔액 차감 + 소유 지급, 트랜잭션) · 소유 목록 갱신.
- **재화 출처**: 경기 보상(실제 결제 아님 → PG·법적 이슈 없음). 실제 결제(PG)는 별도·범위 밖.
- 프로토콜 예약:
  - 업링크 `purchase { partId }` — 서버가 잔액·중복소유 검증 후 트랜잭션.
  - 다운링크 `wallet { balance }` · `inventory { parts:[...] }` 갱신.
  - `catalog`의 `price`·`locked`·소유 검증 활성화.

## Phase 4 — 메일 (inbox)

- **기능 요구**:
  - 메일 **목록 조회**
  - **읽음 / 안읽음** 구분 (안읽음 배지)
  - **첨부 유무** 구분 (첨부 아이콘)
  - 첨부 **수령(claim)** → 파츠/재화가 소유로 이전
  - 메일 **삭제**
- 프로토콜 예약:
  - 업링크: `mail_list`(목록 요청) · `mail_read { id }`(읽음+본문) · `mail_claim { id }`(첨부 수령) · `mail_delete { id }`
  - 다운링크: `mail_list` 응답 `[{ id, subject, read:bool, hasAttachment:bool, sentAt }]` · `mail_detail { id, body, attachments:[...] }` · `mail_badge { unread:int }`
- 데이터(SQLite): `mail(id, user_id, subject, body, read, attachment_ref, claimed, created_at)`

---

## 전방 호환 원칙 (프로토콜)

- 모든 메시지 `t`로 구분, **미지의 `t`는 무시** → 위 타입을 나중에 추가해도 v1과 충돌 없음.
- **공용 데이터**(경기 세계 = `state`/`event`)는 브로드캐스트, **개인 데이터**(지갑·소유·메일)는 해당 세션 유니캐스트.
- `welcome.catalogVersion`으로 카탈로그 갱신 협상.
- 상세 스키마 자리: [02 §9](02-네트워크-프로토콜.md).
