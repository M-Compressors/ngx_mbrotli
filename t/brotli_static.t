#!/usr/bin/perl

# Tests for ngx_http_brotli_static_module.

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

my $t = Test::Nginx->new()->has(qw/http/)->plan(14)
	->write_file_expand('nginx.conf', <<'EOF');

%%TEST_GLOBALS%%

daemon off;

events {
}

http {
    %%TEST_GLOBALS_HTTP%%

    server {
        listen       127.0.0.1:8080;
        server_name  localhost;

        location / {
            brotli_static on;
        }

        location /always/ {
            alias %%TESTDIR%%/;
            brotli_static always;
        }

        location /off/ {
            alias %%TESTDIR%%/;
            brotli_static off;
        }
    }
}

EOF

$t->write_file('file.txt', 'original');
$t->write_file('file.txt.br', 'compressed');
$t->write_file('plain.txt', 'no precompressed variant');
mkdir($t->testdir() . '/dir');
$t->write_file('dir/index.html', 'index');

$t->run();

###############################################################################

my $r;

$r = get('/file.txt', 'br');
like($r, qr/^Content-Encoding: br/mi, 'on: encoding');
like($r, qr/^Content-Length: 10\x0d?$/mi, 'on: length');
like($r, qr/^Content-Type: text\/plain/mi, 'on: type of original');
like($r, qr/\x0d\x0a\x0d\x0acompressed$/, 'on: body');

$r = get('/file.txt', 'gzip');
unlike($r, qr/^Content-Encoding/mi, 'on: not accepted');
like($r, qr/\x0d\x0a\x0d\x0aoriginal$/, 'on: original body');

$r = get('/file.txt', 'br;q=0');
unlike($r, qr/^Content-Encoding/mi, 'on: q=0');

$r = get('/plain.txt', 'br');
like($r, qr/\x0d\x0a\x0d\x0ano precompressed variant$/, 'on: no .br file');

$r = get('/dir/', 'br');
like($r, qr/\x0d\x0a\x0d\x0aindex$/, 'on: directory');

$r = get('/always/file.txt');
like($r, qr/^Content-Encoding: br/mi, 'always: encoding');
like($r, qr/\x0d\x0a\x0d\x0acompressed$/, 'always: body');

$r = get('/off/file.txt', 'br');
unlike($r, qr/^Content-Encoding/mi, 'off');

$r = http(<<EOF);
HEAD /file.txt HTTP/1.1
Host: localhost
Connection: close
Accept-Encoding: br

EOF
like($r, qr/^Content-Encoding: br/mi, 'head: encoding');
like($r, qr/\x0d\x0a\x0d\x0a$/, 'head: no body');

###############################################################################

sub get {
	my ($uri, $ae) = @_;
	my $header = defined $ae ? "Accept-Encoding: $ae\n" : '';
	return http(<<EOF);
GET $uri HTTP/1.1
Host: localhost
Connection: close
$header
EOF
}

###############################################################################
