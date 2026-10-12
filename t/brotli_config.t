#!/usr/bin/perl

# Tests for parsing of the brotli directives.

###############################################################################

use warnings;
use strict;

use Test::More;

BEGIN { use FindBin; chdir($FindBin::Bin); }

use lib 'lib';
use Test::Nginx;

###############################################################################

select STDERR; $| = 1;
select STDOUT; $| = 1;

my $t = Test::Nginx->new()->has(qw/http/)->plan(25);

###############################################################################

my $ok = qr/test is successful/;

# valid configurations

like(check($t, <<'EOF'), $ok, 'all directives');
brotli on;
brotli_types text/plain application/json;
brotli_buffers 16 8k;
brotli_comp_level 11;
brotli_window 16m;
brotli_min_length 256;
brotli_reuse_memory off;
brotli_retention_policy bounded 16m;
brotli_static always;
EOF

like(check($t, 'brotli_retention_policy aggressive;'), $ok, 'aggressive');
like(check($t, 'brotli_retention_policy current_config;'), $ok, 'current_config');
like(check($t, 'brotli_retention_policy release_all;'), $ok, 'release_all');
like(check($t, 'brotli_retention_policy bounded 0;'), $ok, 'bounded 0');

like(check($t, '', 'brotli_retention_policy release_all; brotli_reuse_memory on;'),
	$ok, 'location level');

like(check($t, 'brotli_thread_pool off; brotli_parallel_segment_size 256k; '
		. 'brotli_parallel_tasks 8;'),
	$ok, 'parallel directives');

SKIP: {
skip 'no threads', 3 unless $t->has_module('--with-threads');

like(check($t, 'brotli_thread_pool brotli;', 'brotli_thread_pool off;',
		'thread_pool brotli threads=2;'),
	$ok, 'thread pool');

like(check($t, 'brotli_thread_pool default;'), $ok, 'default thread pool');

like(check($t, 'brotli_thread_pool nope;'),
	qr/unknown thread pool "nope"/,
	'unknown thread pool');

}

SKIP: {
skip 'threads', 1 if $t->has_module('--with-threads');

like(check($t, 'brotli_thread_pool brotli;'),
	qr/"brotli_thread_pool" requires NGINX built with --with-threads/,
	'thread pool without threads');

}

# invalid configurations

like(check($t, 'brotli_retention_policy sometimes;'),
	qr/invalid value "sometimes" in "brotli_retention_policy" directive/,
	'unknown policy');

like(check($t, 'brotli_retention_policy bounded;'),
	qr/"bounded" in "brotli_retention_policy" directive requires a size/,
	'bounded without size');

like(check($t, 'brotli_retention_policy bounded lots;'),
	qr/invalid size "lots" in "brotli_retention_policy" directive/,
	'bounded with invalid size');

like(check($t, 'brotli_retention_policy aggressive 1m;'),
	qr/"aggressive" in "brotli_retention_policy" directive does not take a size/,
	'size for a policy without one');

like(check($t, 'brotli_retention_policy release_all; '
		. 'brotli_retention_policy aggressive;'),
	qr/"brotli_retention_policy" directive is duplicate/,
	'duplicate policy');

like(check($t, 'brotli_reuse_memory maybe;'),
	qr/invalid value "maybe" in "brotli_reuse_memory" directive/,
	'reuse memory flag');

like(check($t, 'brotli_window 3k;'),
	qr/must be 1k, 2k, 4k, 8k, 16k, 32k, 64k, 128k, 256k, 512k, 1m, 2m, 4m, 8m or 16m/,
	'window size');

like(check($t, 'brotli_comp_level 12;'),
	qr/value must be between 0 and 11/,
	'comp level bounds');

like(check($t, 'brotli_static sometimes;'),
	qr/invalid value "sometimes"/,
	'static value');

like(check($t, 'brotli_buffers 4;'),
	qr/invalid number of arguments in "brotli_buffers" directive/,
	'buffers arguments');

like(check($t, 'brotli_thread_pool off; brotli_thread_pool off;'),
	qr/"brotli_thread_pool" directive is duplicate/,
	'duplicate thread pool');

like(check($t, 'brotli_parallel_segment_size 32k;'),
	qr/must be between 64k and 16m/,
	'segment size too small');

like(check($t, 'brotli_parallel_segment_size 32m;'),
	qr/must be between 64k and 16m/,
	'segment size too large');

like(check($t, 'brotli_parallel_tasks 0;'),
	qr/value must be between 1 and 1024/,
	'parallel tasks bounds');

###############################################################################

# Writes nginx.conf with `$http` in the http block, `$location` in a location
# and `$main` in the main context, and returns the output of `nginx -t`. Takes
# `$t` as an argument: a sub closing over it would keep it alive until global
# destruction, too late for its end-of-test checks.
sub check {
	my ($t, $http, $location, $main) = @_;
	$location //= '';
	$main //= '';

	$t->write_file_expand('nginx.conf', <<"EOF");

%%TEST_GLOBALS%%

daemon off;

$main

events {
}

http {
    %%TEST_GLOBALS_HTTP%%

    $http

    server {
        listen       127.0.0.1:8080;
        server_name  localhost;

        location / {
            $location
        }
    }
}

EOF

	my $d = $t->testdir();
	return `$Test::Nginx::NGINX -t -p $d/ -c nginx.conf -e error.log 2>&1`;
}

###############################################################################
