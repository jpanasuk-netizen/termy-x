-- Optional Termy X action-state sync.
-- Idempotent. Safe to run more than once.
-- The app does not embed a project URL or key. Row owner is the signed-in
-- Supabase Auth user (auth.uid()). The anon key alone cannot read or write.

create table if not exists public.x_action_toggles (
    owner uuid not null default auth.uid(),
    x_account text not null,
    kind text not null check (kind in ('like', 'repost', 'follow')),
    item_key text not null,
    confirmed boolean,
    source text,
    verified_at_ms bigint not null default 0,
    seq bigint not null default 0,
    proof text,
    updated_at timestamptz not null default now(),
    primary key (owner, x_account, kind, item_key)
);

create table if not exists public.x_action_receipts (
    owner uuid not null default auth.uid(),
    x_account text not null,
    kind text not null check (kind in ('reply', 'q_tweet', 'post')),
    tweet_id text not null,
    text text not null,
    seq bigint not null,
    at_ms bigint not null,
    updated_at timestamptz not null default now(),
    primary key (owner, x_account, kind, tweet_id, seq)
);

create or replace function public.x_action_keep_newer()
returns trigger
language plpgsql
as $$
begin
    if old.updated_at > new.updated_at then
        return old;
    end if;
    return new;
end;
$$;

drop trigger if exists x_action_toggles_keep_newer on public.x_action_toggles;
create trigger x_action_toggles_keep_newer
    before update on public.x_action_toggles
    for each row
    execute function public.x_action_keep_newer();

drop trigger if exists x_action_receipts_keep_newer on public.x_action_receipts;
create trigger x_action_receipts_keep_newer
    before update on public.x_action_receipts
    for each row
    execute function public.x_action_keep_newer();

alter table public.x_action_toggles enable row level security;
alter table public.x_action_receipts enable row level security;
alter table public.x_action_toggles force row level security;
alter table public.x_action_receipts force row level security;

revoke all on table public.x_action_toggles from public, anon;
revoke all on table public.x_action_receipts from public, anon;
grant select, insert, update on table public.x_action_toggles to authenticated;
grant select, insert, update on table public.x_action_receipts to authenticated;

drop policy if exists x_action_toggles_owner on public.x_action_toggles;
create policy x_action_toggles_owner
    on public.x_action_toggles
    for all
    to authenticated
    using (auth.uid() = owner)
    with check (auth.uid() = owner);

drop policy if exists x_action_receipts_owner on public.x_action_receipts;
create policy x_action_receipts_owner
    on public.x_action_receipts
    for all
    to authenticated
    using (auth.uid() = owner)
    with check (auth.uid() = owner);
