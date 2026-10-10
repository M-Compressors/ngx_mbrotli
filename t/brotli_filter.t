#!/usr/bin/perl

# Tests for ngx_http_brotli_filter_module.

###############################################################################

use warnings;
use strict;

use Test::More;

BEGIN { use FindBin; chdir($FindBin::Bin); }

use lib 'lib';
use Test::Nginx qw/ :DEFAULT http_content /;

###############################################################################

select STDERR; $| = 1;
select STDOUT; $| = 1;

my $t = Test::Nginx->new()->has(qw/http proxy ssi gzip/)->has_daemon('brotli')->plan(46)
	->write_file_expand('nginx.conf', <<'EOF');

%%TEST_GLOBALS%%

daemon off;

events {
}

http {
    %%TEST_GLOBALS_HTTP%%

    types {
        text/html               html;
        text/css                css;
        application/javascript  js;
    }

    log_format ratio '$uri $brotli_ratio';

    # Static files reach body filters as file buffers unless the filter asks
    # for them in memory.
    sendfile on;

    gzip on;
    gzip_types application/javascript;

    server {
        listen       127.0.0.1:8080;
        server_name  localhost;

        brotli on;
        brotli_types text/plain application/javascript;

        access_log %%TESTDIR%%/ratio.log ratio;

        location / {
        }

        location /off/ {
            alias %%TESTDIR%%/;
            brotli off;
        }

        location /q11/ {
            alias %%TESTDIR%%/;
            brotli_comp_level 11;
            brotli_window 16m;
        }

        location /q0/ {
            alias %%TESTDIR%%/;
            brotli_comp_level 0;
            brotli_window 1k;
            brotli_buffers 2 1k;
        }

        location /min/ {
            alias %%TESTDIR%%/;
            brotli_min_length 1000;
        }

        location /noreuse/ {
            alias %%TESTDIR%%/;
            brotli_reuse_memory off;
        }

        location /aggressive/ {
            alias %%TESTDIR%%/;
            brotli_retention_policy aggressive;
        }

        location /current_config/ {
            alias %%TESTDIR%%/;
            brotli_retention_policy current_config;
            brotli_comp_level 9;
        }

        location /release_all/ {
            alias %%TESTDIR%%/;
            brotli_retention_policy release_all;
        }

        location /bounded/ {
            alias %%TESTDIR%%/;
            brotli_retention_policy bounded 64k;
            brotli_comp_level 11;
        }

        location /proxy/ {
            proxy_pass http://127.0.0.1:8081/;
        }

        # Every buffer read from the upstream carries the flush flag.
        location /unbuffered/ {
            proxy_pass http://127.0.0.1:8081/;
            proxy_buffering off;
            proxy_buffer_size 1k;
        }

        location /cond/ {
            alias %%TESTDIR%%/;
            if ($arg_nobr) {
                brotli off;
            }
        }

        location /ssi/ {
            ssi on;
        }

        location = /404 {
            return 404 "not found, not found, not found, not found";
        }

        location = /500 {
            return 500 "internal error, internal error, internal error";
        }

        location = /redirect {
            error_page 404 = /index.html;
            return 404;
        }
    }

    server {
        listen       127.0.0.1:8081;
        server_name  localhost;

        brotli off;

        location / {
            root %%TESTDIR%%;
        }
    }
}

EOF

my $html = '<html>' . join(' ', map { "word$_" } 1 .. 20000) . '</html>';
my $js = "function f() { return 1; }\n" x 5000;

$t->write_file('index.html', $html);
$t->write_file('app.js', $js);
$t->write_file('style.css', 'body { color: red; }' x 100);
$t->write_file('tiny.html', '<p>');
$t->write_file('mid.html', 'X' x 500);
mkdir($t->testdir() . '/ssi');
$t->write_file('ssi/page.html',
	'<html><!--# include virtual="/app.js" -->|'
	. '<!--# include virtual="/mid.html" --></html>');

$t->run();

my $testdir = $t->testdir();

###############################################################################

my $r;

# negotiation

$r = get('/index.html', 'gzip, deflate, br');
like($r, qr/^Content-Encoding: br/mi, 'br accepted');
is(br_body($r), $html, 'html roundtrip');
unlike($r, qr/^Content-Length/mi, 'no content length');
like($r, qr/^ETag: W\//mi, 'weak etag');
unlike($r, qr/^Accept-Ranges/mi, 'no accept ranges');

$r = get('/index.html', 'BR');
like($r, qr/^Content-Encoding: br/mi, 'br case insensitive');

$r = get('/index.html');
unlike($r, qr/^Content-Encoding/mi, 'no accept-encoding');
is(http_content($r), $html, 'no accept-encoding body');

$r = get('/index.html', 'gzip, br;q=0');
unlike($r, qr/^Content-Encoding: br/mi, 'br q=0 rejected');

$r = get('/index.html', 'br;q=0.001');
like($r, qr/^Content-Encoding: br/mi, 'br q=0.001 accepted');

$r = get('/index.html', 'brotli');
unlike($r, qr/^Content-Encoding/mi, 'br token boundary');

# types and lengths

$r = get('/app.js', 'br');
like($r, qr/^Content-Encoding: br/mi, 'brotli_types match');
is(br_body($r), $js, 'js roundtrip');

$r = get('/style.css', 'br');
unlike($r, qr/^Content-Encoding/mi, 'brotli_types mismatch');

$r = get('/tiny.html', 'br');
unlike($r, qr/^Content-Encoding/mi, 'default min length');

$r = get('/min/mid.html', 'br');
unlike($r, qr/^Content-Encoding/mi, 'brotli_min_length');

$r = get('/mid.html', 'br');
like($r, qr/^Content-Encoding: br/mi, 'above min length');

# configuration

$r = get('/off/index.html', 'br');
unlike($r, qr/^Content-Encoding/mi, 'brotli off');

$r = get('/q11/index.html', 'br');
is(br_body($r), $html, 'comp level 11, 16m window');

$r = get('/q0/index.html', 'br');
is(br_body($r), $html, 'comp level 0, 1k window, small buffers');

$r = get('/cond/index.html', 'br');
like($r, qr/^Content-Encoding: br/mi, 'if: brotli inherited');

$r = get('/cond/index.html?nobr=1', 'br');
unlike($r, qr/^Content-Encoding/mi, 'if: brotli off');

# memory reuse and retention policies: each request twice, so that the
# second one gets an encoder returned to the cache by the first

for my $prefix (qw/noreuse aggressive current_config release_all bounded/) {
	for my $n (1, 2) {
		$r = get("/$prefix/index.html", 'br');
		is(br_body($r), $html, "$prefix $n");
	}
}

# response kinds

$r = get('/proxy/index.html', 'br');
is(br_body($r), $html, 'proxied response');

$r = get('/unbuffered/index.html', 'br');
is(br_body($r), $html, 'unbuffered proxied response');

$r = get('/404', 'br');
like($r, qr/^Content-Encoding: br/mi, '404 compressed');
is(br_body($r), 'not found, not found, not found, not found', '404 body');

$r = get('/500', 'br');
unlike($r, qr/^Content-Encoding/mi, '500 not compressed');

$r = get('/redirect', 'br');
is(br_body($r), $html, 'error_page redirect');

$r = http(<<EOF);
HEAD /index.html HTTP/1.1
Host: localhost
Connection: close
Accept-Encoding: br

EOF
like($r, qr/^Content-Encoding: br/mi, 'head');
like($r, qr/\x0d\x0a\x0d\x0a$/, 'head no body');

$r = get('/ssi/page.html', 'br');
is(br_body($r), '<html>' . $js . '|' . 'X' x 500 . '</html>', 'ssi subrequests');

# gzip coexistence

$r = get('/app.js', 'gzip, br');
like($r, qr/^Content-Encoding: br/mi, 'br preferred over gzip');
unlike($r, qr/^Content-Encoding: gzip/mi, 'no double encoding');

$r = get('/app.js', 'gzip');
like($r, qr/^Content-Encoding: gzip/mi, 'gzip when br not accepted');

# $brotli_ratio

$t->stop();

my $log = $t->read_file('ratio.log');
like($log, qr!^/app\.js \d+\.\d\d$!m, 'brotli_ratio');
like($log, qr!^/style\.css -$!m, 'brotli_ratio not found');

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

sub br_body {
	my ($r) = @_;
	my $body = http_content($r);
	return undef unless defined $body;

	my $file = "$testdir/response.br";
	open my $fh, '>', $file or die "Can't create $file: $!";
	binmode $fh;
	print $fh $body;
	close $fh;

	return `brotli -dc $file`;
}

###############################################################################
